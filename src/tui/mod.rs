//! The notes pane and the peek popup.

pub mod app;
pub mod ui;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::config::{self, Config};
use crate::index::{self, Index};
use crate::render::Theme;
use app::App;

enum AppEvent {
    Term(Event),
    Batch(std::collections::BTreeSet<String>),
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

pub fn run(root_arg: Option<&str>) -> Result<(), String> {
    let loaded = load(root_arg);
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = match loaded {
        Ok((app, cache)) => event_loop(&mut terminal, app, cache),
        Err(e) => show_error(&mut terminal, &e),
    };
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn load(root_arg: Option<&str>) -> Result<(App, Option<PathBuf>), String> {
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
    let mut app = App::new(index, label, root.send_allow, theme);
    app.status = status;
    Ok((app, cache))
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: App,
    cache: Option<PathBuf>,
) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let input = tx.clone();
    thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if input.send(AppEvent::Term(ev)).is_err() {
                return;
            }
        }
    });
    let ignore = cache.as_deref().and_then(|c| c.parent()).map(PathBuf::from);
    match crate::watch::watch(&app.index.root, ignore.as_deref()) {
        Ok(watch) => {
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

    loop {
        terminal
            .draw(|f| ui::draw(f, &mut app))
            .map_err(|e| format!("draw: {e}"))?;
        let Ok(ev) = rx.recv() else {
            break;
        };
        match ev {
            AppEvent::Term(Event::Key(k)) => app.key(k),
            AppEvent::Term(Event::Mouse(m)) => app.mouse(m),
            AppEvent::Term(_) => {}
            AppEvent::Batch(b) => app.batch(&b),
        }
        if app.quit {
            break;
        }
    }
    if let Some(c) = &cache {
        let _ = app.index.save_cache(c);
    }
    Ok(())
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
