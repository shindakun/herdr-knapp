//! The notes pane and the peek popup.

pub mod app;
pub mod ui;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::config::{self, Config};
use crate::index::{self, Index};
use crate::render::Theme;
use crate::scan::Kind;
use crate::search::{self, Match};
use app::{App, Effect};

enum AppEvent {
    Term(Event),
    Batch(std::collections::BTreeSet<String>),
    Results(u64, Vec<Match>),
    Agents(Result<Vec<crate::herdr::Agent>, String>, Option<String>),
    Sent(Result<String, String>),
}

/// `HERDR_PLUGIN_STATE_DIR/last-agent`: the pane id last sent to.
fn last_agent_path() -> Option<PathBuf> {
    var("HERDR_PLUGIN_STATE_DIR").map(|d| PathBuf::from(d).join("last-agent"))
}

/// The workspace's agents with herdr-shaped pane ids.
fn workspace_agents() -> Result<Vec<crate::herdr::Agent>, String> {
    let ctx = crate::herdr::Context::from_env().ok_or("sending needs herdr")?;
    let workspace = ctx.workspace_id.ok_or("herdr gave no workspace id")?;
    Ok(crate::herdr::agents()?
        .into_iter()
        .filter(|a| a.workspace_id == workspace && crate::send::valid_pane_id(&a.pane_id))
        .collect())
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The directory relative roots resolve against: `KNAPP_CWD`, else the
/// workspace herdr launched the pane in (see `herdr::workspace_dir`), else
/// `None` under herdr (the process cwd is the plugin root, never notes), else
/// the current directory.
fn base_dir() -> Result<Option<PathBuf>, String> {
    if let Some(dir) = var("KNAPP_CWD") {
        return Ok(Some(PathBuf::from(dir)));
    }
    if let Some(ctx) = crate::herdr::Context::from_env() {
        let agents = crate::herdr::agents().unwrap_or_default();
        return Ok(crate::herdr::workspace_dir(&ctx, &agents));
    }
    std::env::current_dir()
        .map(Some)
        .map_err(|e| format!("current directory: {e}"))
}

struct Loaded {
    app: App,
    cache: Option<PathBuf>,
    editor: String,
    exclude: Vec<String>,
}

pub fn run(root_arg: Option<&str>) -> Result<(), String> {
    let loaded = load(root_arg);
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = match loaded {
        Ok(loaded) => event_loop(&mut terminal, loaded),
        Err(e) => show_error(&mut terminal, &e),
    };
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn load(root_arg: Option<&str>) -> Result<Loaded, String> {
    let config = Config::load()?;
    let root = match base_dir()? {
        Some(base) => config.pick_root(root_arg, &base, &base)?,
        None => {
            let first = config.roots(Path::new("/")).into_iter().next();
            match (root_arg, first) {
                (Some(arg), _) => config.pick_root(Some(arg), Path::new("/"), Path::new("/"))?,
                (None, Some(root)) => root,
                (None, None) => {
                    return Err("no notes root: this workspace has no agent to take a \
                         directory from, and the config names no [[root]]"
                        .into())
                }
            }
        }
    };
    let canonical = config::canonical(&root.path);
    let cache = config::cache_dir().map(|d| index::cache_path(&d, &canonical));
    let (index, stats) = Index::load(&root.path, &config.exclude, cache.as_deref())?;
    let mut status = None;
    if let (Some(c), true) = (&cache, stats.stale) {
        if let Err(e) = index.save_cache(c) {
            status = Some(format!("cache not written: {e}"));
        }
    }
    let label = root
        .name
        .clone()
        .unwrap_or_else(|| canonical.display().to_string());
    let theme = Theme {
        color: var("NO_COLOR").is_none(),
    };
    let editor = crate::editor::choose(
        &config.editor,
        var("VISUAL").as_deref(),
        var("EDITOR").as_deref(),
    );
    let mut app = App::new(index, label, root.send_allow, theme);
    app.status = status;
    app.send_max_bytes = config.send_max_bytes;
    app.graph_hops = config.graph_hops;
    Ok(Loaded {
        app,
        cache,
        editor,
        exclude: config.exclude,
    })
}

/// The input thread polls so it can stop reading the terminal while the
/// editor runs; otherwise it would take the editor's keystrokes.
struct Input {
    paused: Arc<AtomicBool>,
    idle: Arc<AtomicBool>,
}

impl Input {
    fn spawn(tx: mpsc::Sender<AppEvent>) -> Self {
        let paused = Arc::new(AtomicBool::new(false));
        let idle = Arc::new(AtomicBool::new(false));
        let (p, i) = (paused.clone(), idle.clone());
        thread::spawn(move || loop {
            if p.load(Ordering::SeqCst) {
                i.store(true, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            i.store(false, Ordering::SeqCst);
            match event::poll(Duration::from_millis(50)) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if tx.send(AppEvent::Term(ev)).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        });
        Input { paused, idle }
    }

    /// Stops reading and waits until the thread is between polls.
    fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !self.idle.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, loaded: Loaded) -> Result<(), String> {
    let Loaded {
        mut app,
        cache,
        editor,
        exclude,
    } = loaded;
    let (tx, rx) = mpsc::channel();
    let input = Input::spawn(tx.clone());
    let ignore = cache.as_deref().and_then(|c| c.parent()).map(PathBuf::from);
    match crate::watch::watch(&app.index.root, ignore.as_deref()) {
        Ok(watch) => {
            let tx = tx.clone();
            thread::spawn(move || {
                while let Some(batch) = watch.next_batch() {
                    if tx.send(AppEvent::Batch(batch)).is_err() {
                        return;
                    }
                }
            });
        }
        Err(e) => app.status = Some(format!("not watching for changes: {e}")),
    }
    let rg = search::find_rg();
    let graphics = crate::herdr::Graphics::from_env();
    let mut sync = Sync::default();
    query_graphics(&graphics, &mut app);
    let mut search_cancel: Option<Arc<AtomicBool>> = None;

    loop {
        terminal
            .draw(|f| ui::draw(f, &mut app))
            .map_err(|e| format!("draw: {e}"))?;
        if let Some(g) = &graphics {
            if let Err(e) = sync.apply(g, &app.layers, app.cell_px) {
                // feature_disabled and the like: no images for this session.
                app.set_cell_px(None);
                app.status = Some(format!("pane graphics off: {e}"));
            }
        }
        let Ok(ev) = rx.recv() else {
            break;
        };
        match ev {
            AppEvent::Term(Event::Key(k)) => app.key(k),
            AppEvent::Term(Event::Mouse(m)) => app.mouse(m),
            AppEvent::Term(Event::Resize(..)) => query_graphics(&graphics, &mut app),
            AppEvent::Term(_) => {}
            AppEvent::Batch(b) => app.batch(&b),
            AppEvent::Results(generation, batch) => app.results(generation, batch),
            AppEvent::Agents(result, last) => app.agents(result, last),
            AppEvent::Sent(result) => app.sent(result),
        }
        for effect in app.take_effects() {
            match effect {
                Effect::Edit { path, line } => {
                    input.pause();
                    let result = run_editor(terminal, &editor, &path, line, &app.index.root);
                    input.resume();
                    if let Err(e) = result {
                        app.status = Some(format!("editor: {e}"));
                    }
                }
                Effect::ListAgents => {
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let last = last_agent_path()
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .map(|s| s.trim().to_string());
                        let _ = tx.send(AppEvent::Agents(workspace_agents(), last));
                    });
                }
                Effect::Send { pane, agent, text } => {
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let result = crate::herdr::prompt(&pane, &text).map(|()| {
                            if let Some(path) = last_agent_path() {
                                let _ = std::fs::write(path, &pane);
                            }
                            agent
                        });
                        let _ = tx.send(AppEvent::Sent(result));
                    });
                }
                Effect::Copy(text) => {
                    let mut out = std::io::stdout();
                    let _ = out.write_all(crate::editor::osc52(&text).as_bytes());
                    let _ = out.flush();
                }
                Effect::Search { generation, query } => {
                    if let Some(c) = search_cancel.take() {
                        c.store(true, Ordering::Relaxed);
                    }
                    let cancel = Arc::new(AtomicBool::new(false));
                    search_cancel = Some(cancel.clone());
                    let notes: Vec<String> = app
                        .index
                        .files
                        .iter()
                        .filter(|f| f.kind == Kind::Note && !f.excluded)
                        .map(|f| f.rel.clone())
                        .collect();
                    let (root, exclude, rg, tx) = (
                        app.index.root.clone(),
                        exclude.clone(),
                        rg.clone(),
                        tx.clone(),
                    );
                    thread::spawn(move || {
                        let mut batch = Vec::new();
                        let mut last = Instant::now();
                        let _ = search::search(
                            &root,
                            &query,
                            &exclude,
                            &notes,
                            rg.as_deref(),
                            &cancel,
                            |m| {
                                batch.push(m);
                                if batch.len() >= 50 || last.elapsed() > Duration::from_millis(100)
                                {
                                    last = Instant::now();
                                    return tx
                                        .send(AppEvent::Results(
                                            generation,
                                            std::mem::take(&mut batch),
                                        ))
                                        .is_ok();
                                }
                                true
                            },
                        );
                        if !batch.is_empty() {
                            let _ = tx.send(AppEvent::Results(generation, batch));
                        }
                    });
                }
            }
        }
        if app.quit {
            break;
        }
    }
    if let Some(g) = &graphics {
        let _ = sync.apply(g, &[], None);
    }
    if let Some(c) = &cache {
        let _ = app.index.save_cache(c);
    }
    Ok(())
}

fn query_graphics(graphics: &Option<crate::herdr::Graphics>, app: &mut App) {
    let Some(g) = graphics else {
        return;
    };
    match g.info() {
        Ok(info) if info.cell_px.0 > 0 && info.cell_px.1 > 0 => app.set_cell_px(Some(info.cell_px)),
        Ok(_) => app.set_cell_px(None),
        Err(e) => {
            app.set_cell_px(None);
            app.status = Some(format!("pane graphics off: {e}"));
        }
    }
}

type CellPx = (u32, u32);
/// A PNG and its size in pixels.
type Png = (Vec<u8>, (u32, u32));

/// What herdr has been told: layer name to (content key, placement). A layer
/// is sent again only when its content or placement changes.
#[derive(Default)]
struct Sync {
    /// Decoded images, for cutting the band that is on screen.
    decoded: HashMap<PathBuf, tiny_skia::Pixmap>,
    sent: HashMap<String, (String, crate::herdr::Placement)>,
    pngs: HashMap<(String, CellPx), Png>,
}

impl Sync {
    /// The PNG for a layer's visible band.
    fn render(&mut self, layer: &app::Layer, px: CellPx) -> Result<Vec<u8>, String> {
        match &layer.content {
            app::LayerContent::Canvas(spec) => crate::graph::canvas(spec, px, layer.band.clone()),
            app::LayerContent::Image { path, rows } => {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                if layer.band.start == 0 && layer.band.end >= *rows {
                    return Ok(bytes);
                }
                if !self.decoded.contains_key(path) {
                    if self.decoded.len() >= 8 {
                        self.decoded.clear();
                    }
                    let pm = tiny_skia::Pixmap::decode_png(&bytes).map_err(|e| e.to_string())?;
                    self.decoded.insert(path.clone(), pm);
                }
                crate::graph::crop_rows(&self.decoded[path], *rows, layer.band.clone())
            }
        }
    }

    fn apply(
        &mut self,
        g: &crate::herdr::Graphics,
        layers: &[app::Layer],
        cell_px: Option<(u32, u32)>,
    ) -> Result<(), String> {
        let wanted: Vec<&app::Layer> = match cell_px {
            Some(_) => layers.iter().collect(),
            None => Vec::new(),
        };
        let gone: Vec<String> = self
            .sent
            .keys()
            .filter(|name| !wanted.iter().any(|l| &l.name == *name))
            .cloned()
            .collect();
        for name in gone {
            self.sent.remove(&name);
            g.clear(&name)?;
        }
        for layer in wanted {
            let now = (layer.key.clone(), layer.at);
            if self.sent.get(&layer.name) == Some(&now) {
                continue;
            }
            let px = cell_px.expect("graphics are on");
            let (png, size) = match self.pngs.get(&(layer.key.clone(), px)) {
                Some(done) => done.clone(),
                None => {
                    // A picture that cannot be drawn is left out; only herdr
                    // refusing a call turns graphics off.
                    let Ok(png) = self.render(layer, px) else {
                        continue;
                    };
                    let Some(size) = crate::herdr::png_size(&png) else {
                        continue;
                    };
                    // Keys change with every resize, scroll, and refresh.
                    if self.pngs.len() >= 32 {
                        self.pngs.clear();
                    }
                    self.pngs
                        .insert((layer.key.clone(), px), (png.clone(), size));
                    (png, size)
                }
            };
            g.set(&layer.name, &png, size, layer.at)?;
            self.sent.insert(layer.name.clone(), now);
        }
        Ok(())
    }
}

/// Suspends the TUI, runs the editor in the root, and restores the TUI. The
/// input thread must already be paused.
fn run_editor(
    terminal: &mut ratatui::DefaultTerminal,
    editor: &str,
    path: &Path,
    line: u32,
    root: &Path,
) -> Result<(), String> {
    let argv = crate::editor::command(editor, path, line);
    let mut out = std::io::stdout();
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    let _ = terminal.show_cursor();
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .status();
    let _ = enable_raw_mode();
    let _ = execute!(out, EnterAlternateScreen, EnableMouseCapture);
    let _ = terminal.clear();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{} exited with {s}", argv[0])),
        Err(e) => Err(format!("{}: {e}", argv[0])),
    }
}

/// A root that fails to load: say why, and wait for `q`.
fn show_error(terminal: &mut ratatui::DefaultTerminal, message: &str) -> Result<(), String> {
    loop {
        terminal
            .draw(|f| {
                let lines = vec![
                    Line::from(format!("knapp: {message}")),
                    Line::default(),
                    Line::from("q quits"),
                ];
                f.render_widget(Paragraph::new(lines), f.area());
            })
            .map_err(|e| format!("draw: {e}"))?;
        if let Ok(Event::Key(k)) = event::read() {
            if matches!(
                k.code,
                event::KeyCode::Char('q') | event::KeyCode::Esc | event::KeyCode::Enter
            ) || (k.code == event::KeyCode::Char('c')
                && k.modifiers.contains(event::KeyModifiers::CONTROL))
            {
                return Err(message.to_string());
            }
        }
    }
}
