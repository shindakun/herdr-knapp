//! Pane state and key handling. No terminal access, so tests can drive it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::rc::Rc;

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListState;

use std::path::PathBuf;

use crate::index::{block_fragment, key, FileId, Index, Resolved};
use crate::render::{self, LinkState, Theme};
use crate::scan::Kind;
use crate::search::{Match, MAX_RESULTS};

/// Work the event loop does for the app: processes, the terminal, threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Edit { path: PathBuf, line: u32 },
    Copy(String),
    Search { generation: u64, query: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Tree,
    Backlinks,
    Forward,
    Tags,
    Unresolved,
    Orphans,
    Recent,
    Search,
}

impl Mode {
    pub const ALL: [Mode; 8] = [
        Mode::Tree,
        Mode::Backlinks,
        Mode::Forward,
        Mode::Tags,
        Mode::Unresolved,
        Mode::Orphans,
        Mode::Recent,
        Mode::Search,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Tree => "Tree",
            Mode::Backlinks => "Backlinks",
            Mode::Forward => "Forward",
            Mode::Tags => "Tags",
            Mode::Unresolved => "Unresolved",
            Mode::Orphans => "Orphans",
            Mode::Recent => "Recent",
            Mode::Search => "Search",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Detail,
}

/// What the detail panel shows. Notes are held by path: file ids change on
/// every refresh.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    Summary,
    Note(String),
    Unresolved(String),
    /// An ambiguous target, by its `key()`.
    Ambiguous(String),
    Attachment(String),
}

struct Visit {
    page: Page,
    scroll: usize,
}

#[derive(Debug, Clone)]
pub enum HitTarget {
    /// A link in the open note, by index into its `Parsed::links`.
    Link(usize),
    Open {
        rel: String,
        line: u32,
    },
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub line: usize,
    pub cols: Range<u16>,
    pub target: HitTarget,
}

pub struct Detail {
    pub title: String,
    pub lines: Vec<Line<'static>>,
    pub source_line: Vec<u32>,
    pub hits: Vec<Hit>,
}

#[derive(Debug, Clone)]
pub enum RowAction {
    Folder(String),
    Open {
        rel: String,
        line: Option<u32>,
        link: Option<usize>,
    },
    Follow(usize),
    Tag(String),
    Show(Page),
    Nothing,
}

#[derive(Clone)]
pub struct Row {
    pub line: Line<'static>,
    pub action: RowAction,
}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<(String, String), Dir>,
    files: Vec<(String, String)>,
}

pub struct App {
    pub index: Index,
    pub root_label: String,
    pub send_allow: Vec<String>,
    pub theme: Theme,
    pub mode: Mode,
    pub focus: Focus,
    pub list: ListState,
    pub scroll: usize,
    /// Index into the current detail's `hits`.
    pub selected_hit: Option<usize>,
    pub fold_frontmatter: bool,
    pub status: Option<String>,
    pub help: bool,
    pub quit: bool,
    /// Set by drawing; used for paging and the mouse.
    pub list_area: Rect,
    pub detail_area: Rect,
    pub narrow: bool,
    expanded: BTreeSet<String>,
    expanded_tags: BTreeSet<String>,
    history: Vec<Visit>,
    cursor: usize,
    /// A source line to scroll to, and a link to select, once the page is
    /// rendered at a known width.
    jump: Option<(u32, Option<usize>)>,
    details: HashMap<(Page, u16, bool), Rc<Detail>>,
    rows: Option<Rc<Vec<Row>>>,
    tree: Dir,
    effects: Vec<Effect>,
    /// The search query line: open while typing.
    pub query: String,
    pub query_open: bool,
    generation: u64,
    results: Vec<Match>,
}

impl App {
    pub fn new(index: Index, root_label: String, send_allow: Vec<String>, theme: Theme) -> Self {
        let mut app = Self {
            index,
            root_label,
            send_allow,
            theme,
            mode: Mode::Tree,
            focus: Focus::List,
            list: ListState::default().with_selected(Some(0)),
            scroll: 0,
            selected_hit: None,
            fold_frontmatter: true,
            status: None,
            help: false,
            quit: false,
            list_area: Rect::default(),
            detail_area: Rect::default(),
            narrow: false,
            expanded: BTreeSet::new(),
            expanded_tags: BTreeSet::new(),
            history: vec![Visit {
                page: Page::Summary,
                scroll: 0,
            }],
            cursor: 0,
            jump: None,
            details: HashMap::new(),
            rows: None,
            tree: Dir::default(),
            effects: Vec::new(),
            query: String::new(),
            query_open: false,
            generation: 0,
            results: Vec::new(),
        };
        app.build_tree();
        app
    }

    pub fn page(&self) -> &Page {
        &self.history[self.cursor].page
    }

    fn open_id(&self) -> Option<FileId> {
        match self.page() {
            Page::Note(rel) => self.index.id(rel),
            _ => None,
        }
    }

    // ---- list rows ----

    fn build_tree(&mut self) {
        let mut root = Dir::default();
        for f in self.index.files.iter().filter(|f| !f.excluded) {
            let mut dir = &mut root;
            let mut parts: Vec<&str> = f.rel.split('/').collect();
            let name = parts.pop().expect("a path has a name");
            let mut path = String::new();
            for part in parts {
                path.push_str(part);
                path.push('/');
                dir = dir
                    .dirs
                    .entry((part.to_lowercase(), part.to_string()))
                    .or_default();
            }
            dir.files.push((name.to_string(), f.rel.clone()));
        }
        fn sort(d: &mut Dir) {
            d.files.sort_by(|a, b| {
                a.0.to_lowercase()
                    .cmp(&b.0.to_lowercase())
                    .then(a.0.cmp(&b.0))
            });
            d.dirs.values_mut().for_each(sort);
        }
        sort(&mut root);
        self.tree = root;
        self.rows = None;
    }

    pub fn rows(&mut self) -> Rc<Vec<Row>> {
        if let Some(rows) = &self.rows {
            return rows.clone();
        }
        let rows = Rc::new(match self.mode {
            Mode::Tree => self.tree_rows(),
            Mode::Backlinks => self.backlink_rows(),
            Mode::Forward => self.forward_rows(),
            Mode::Tags => self.tag_rows(),
            Mode::Unresolved => self.unresolved_rows(),
            Mode::Orphans => self.orphan_rows(),
            Mode::Recent => self.recent_rows(),
            Mode::Search => self.search_rows(),
        });
        self.rows = Some(rows.clone());
        rows
    }

    fn tree_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let dim = self.theme.dim();
        fn walk(app: &App, d: &Dir, prefix: &str, depth: usize, rows: &mut Vec<Row>, dim: Style) {
            let indent = "  ".repeat(depth);
            for ((_, name), sub) in &d.dirs {
                let path = format!("{prefix}{name}/");
                let open = app.expanded.contains(&path);
                let mark = if open { "▾" } else { "▸" };
                rows.push(Row {
                    line: Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(format!("{mark} "), dim),
                        Span::styled(
                            format!("{name}/"),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    action: RowAction::Folder(path.clone()),
                });
                if open {
                    walk(app, sub, &path, depth + 1, rows, dim);
                }
            }
            for (name, rel) in &d.files {
                let is_note = app
                    .index
                    .id(rel)
                    .is_some_and(|id| app.index.files[id].kind == Kind::Note);
                let style = if is_note { Style::new() } else { dim };
                rows.push(Row {
                    line: Line::from(vec![
                        Span::raw(format!("{indent}  ")),
                        Span::styled(name.clone(), style),
                    ]),
                    action: RowAction::Open {
                        rel: rel.clone(),
                        line: None,
                        link: None,
                    },
                });
            }
        }
        walk(self, &self.tree, "", 0, &mut rows, dim);
        rows
    }

    fn backlink_rows(&self) -> Vec<Row> {
        let Some(id) = self.open_id() else {
            return vec![self.message_row("Open a note to see its backlinks.")];
        };
        let mut hits: Vec<(String, u32, usize)> = self.index.back[id]
            .iter()
            .map(|&(source, pos)| {
                (
                    self.index.files[source].rel.clone(),
                    self.index.links(source)[pos].line,
                    pos,
                )
            })
            .collect();
        hits.sort();
        hits.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        if hits.is_empty() {
            return vec![self.message_row("No backlinks.")];
        }
        let mut texts: HashMap<String, Vec<String>> = HashMap::new();
        hits.into_iter()
            .map(|(rel, line, pos)| {
                let lines = texts.entry(rel.clone()).or_insert_with(|| {
                    std::fs::read_to_string(self.index.root.join(&rel))
                        .unwrap_or_default()
                        .split('\n')
                        .map(str::to_string)
                        .collect()
                });
                let context = lines.get(line as usize - 1).map_or("", |l| l.trim());
                Row {
                    line: Line::from(vec![
                        Span::raw(format!("{rel}:{line}  ")),
                        Span::styled(context.to_string(), self.theme.dim()),
                    ]),
                    action: RowAction::Open {
                        rel,
                        line: Some(line),
                        link: Some(pos),
                    },
                }
            })
            .collect()
    }

    fn forward_rows(&self) -> Vec<Row> {
        let Some(id) = self.open_id() else {
            return vec![self.message_row("Open a note to see its links.")];
        };
        let links = self.index.links(id);
        if links.is_empty() {
            return vec![self.message_row("No links.")];
        }
        links
            .iter()
            .zip(&self.index.forward[id])
            .enumerate()
            .map(|(i, (link, state))| {
                let (mark, target) = match state {
                    Resolved::File { id, .. } => ("  ", self.index.files[*id].rel.clone()),
                    Resolved::Ambiguous { pick, others, .. } => (
                        "? ",
                        format!("{} (+{})", self.index.files[*pick].rel, others.len()),
                    ),
                    Resolved::Unresolved => ("✗ ", "no note".to_string()),
                };
                Row {
                    line: Line::from(vec![
                        Span::styled(mark, self.theme.dim()),
                        Span::styled(link.written.clone(), self.theme.link(state)),
                        Span::styled(format!("  → {target}"), self.theme.dim()),
                    ]),
                    action: RowAction::Follow(i),
                }
            })
            .collect()
    }

    fn open_row(&self, rel: &str, prefix: String) -> Row {
        Row {
            line: Line::from(vec![
                Span::styled(prefix, self.theme.dim()),
                Span::raw(rel.to_string()),
            ]),
            action: RowAction::Open {
                rel: rel.to_string(),
                line: None,
                link: None,
            },
        }
    }

    fn tag_rows(&self) -> Vec<Row> {
        fn walk(app: &App, nodes: &[crate::index::TagNode], depth: usize, rows: &mut Vec<Row>) {
            let indent = "  ".repeat(depth);
            for n in nodes {
                let open = app.expanded_tags.contains(&n.key);
                let mark = if open { "▾" } else { "▸" };
                rows.push(Row {
                    line: Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(format!("{mark} #"), app.theme.dim()),
                        Span::styled(
                            n.name().to_string(),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("  {}", n.count), app.theme.dim()),
                    ]),
                    action: RowAction::Tag(n.key.clone()),
                });
                if open {
                    walk(app, &n.children, depth + 1, rows);
                    for &id in &n.notes {
                        rows.push(app.open_row(&app.index.files[id].rel, format!("{indent}    ")));
                    }
                }
            }
        }
        let tags = self.index.tags();
        if tags.is_empty() {
            return vec![self.message_row("No tags.")];
        }
        let mut rows = Vec::new();
        walk(self, &tags, 0, &mut rows);
        rows
    }

    fn unresolved_rows(&self) -> Vec<Row> {
        let groups = self.index.unresolved();
        if groups.is_empty() {
            return vec![self.message_row("Every link resolves.")];
        }
        groups
            .iter()
            .map(|g| {
                let (mark, page, state) = if g.ambiguous {
                    ("? ", Page::Ambiguous(g.key.clone()), LinkState::Ambiguous)
                } else {
                    (
                        "✗ ",
                        Page::Unresolved(g.shown.clone()),
                        LinkState::Unresolved,
                    )
                };
                Row {
                    line: Line::from(vec![
                        Span::styled(format!("{mark}{:>3}  ", g.sources.len()), self.theme.dim()),
                        Span::styled(g.shown.clone(), self.theme.link_state(state)),
                    ]),
                    action: RowAction::Show(page),
                }
            })
            .collect()
    }

    fn orphan_rows(&self) -> Vec<Row> {
        let orphans = self.index.orphans();
        if orphans.is_empty() {
            return vec![self.message_row("No orphans.")];
        }
        orphans
            .iter()
            .map(|&id| self.open_row(&self.index.files[id].rel, String::new()))
            .collect()
    }

    fn recent_rows(&self) -> Vec<Row> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let mut notes: Vec<&crate::index::File> = self
            .index
            .files
            .iter()
            .filter(|f| f.kind == Kind::Note && !f.excluded)
            .collect();
        notes.sort_by(|a, b| b.mtime_ns.cmp(&a.mtime_ns).then(a.rel.cmp(&b.rel)));
        notes
            .iter()
            .map(|f| {
                let secs = (now.saturating_sub(f.mtime_ns) / 1_000_000_000) as u64;
                self.open_row(&f.rel, format!("{:>4}  ", age(secs)))
            })
            .collect()
    }

    fn search_rows(&self) -> Vec<Row> {
        if self.query.is_empty() {
            return vec![self.message_row("/ to search.")];
        }
        if self.results.is_empty() {
            return vec![self.message_row("No matches.")];
        }
        let hit = Style::new().add_modifier(Modifier::REVERSED);
        self.results
            .iter()
            .map(|m| {
                let lead = m.text.len() - m.text.trim_start().len();
                let mut spans = vec![Span::raw(format!("{}:{}  ", m.rel, m.line))];
                let mut pos = lead;
                for r in &m.ranges {
                    if r.start < pos || r.end > m.text.len() {
                        continue;
                    }
                    spans.push(Span::styled(
                        m.text[pos..r.start].to_string(),
                        self.theme.dim(),
                    ));
                    spans.push(Span::styled(m.text[r.clone()].to_string(), hit));
                    pos = r.end;
                }
                spans.push(Span::styled(m.text[pos..].to_string(), self.theme.dim()));
                Row {
                    line: Line::from(spans),
                    action: RowAction::Open {
                        rel: m.rel.clone(),
                        line: Some(m.line),
                        link: None,
                    },
                }
            })
            .collect()
    }

    /// Results from the search thread. Stale generations are dropped, and so
    /// is any path that is not a visible, indexed note.
    pub fn results(&mut self, generation: u64, batch: Vec<Match>) {
        if generation != self.generation {
            return;
        }
        for m in batch {
            let visible = self.index.id(&m.rel).is_some_and(|id| {
                let f = &self.index.files[id];
                f.kind == Kind::Note && !f.excluded
            });
            if visible && self.results.len() < MAX_RESULTS {
                self.results.push(m);
            }
        }
        if self.mode == Mode::Search {
            self.rows = None;
        }
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn query_changed(&mut self) {
        self.generation += 1;
        self.results.clear();
        self.rows = None;
        self.list.select(Some(0));
        if !self.query.is_empty() {
            self.effects.push(Effect::Search {
                generation: self.generation,
                query: self.query.clone(),
            });
        }
    }

    fn query_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Enter | KeyCode::Esc => self.query_open = false,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.query_changed();
            }
            KeyCode::Backspace => {
                if self.query.pop().is_some() {
                    self.query_changed();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.query_changed();
            }
            _ => {}
        }
    }

    /// Opens the note in the editor at the selected link's line, else the
    /// top visible line.
    fn edit(&mut self) {
        let Page::Note(rel) = self.page().clone() else {
            self.status = Some("o opens notes only".into());
            return;
        };
        let width = self.detail_width();
        let detail = self.detail(width, self.detail_height() as u16);
        let row = self
            .selected_hit
            .and_then(|i| detail.hits.get(i))
            .map_or(self.scroll, |h| h.line);
        let line = detail.source_line.get(row).copied().unwrap_or(1);
        self.effects.push(Effect::Edit {
            path: self.index.root.join(&rel),
            line,
        });
    }

    fn copy(&mut self, wikilink: bool) {
        let text = match self.page().clone() {
            Page::Note(rel) | Page::Attachment(rel) => match self.index.id(&rel) {
                Some(id) if wikilink => format!("[[{}]]", self.index.shortest_link(id)),
                _ => self.index.root.join(&rel).display().to_string(),
            },
            Page::Unresolved(target) | Page::Ambiguous(target) => target,
            Page::Summary => {
                self.status = Some("open a note to copy it".into());
                return;
            }
        };
        self.status = Some(format!("copied: {text}"));
        self.effects.push(Effect::Copy(text));
    }

    fn message_row(&self, text: &str) -> Row {
        Row {
            line: Line::from(Span::styled(text.to_string(), self.theme.dim())),
            action: RowAction::Nothing,
        }
    }

    // ---- the detail panel ----

    /// The current page rendered at `width`, with any pending jump applied.
    pub fn detail(&mut self, width: u16, height: u16) -> Rc<Detail> {
        let page = self.page().clone();
        let cache_key = (page.clone(), width, self.fold_frontmatter);
        let detail = match self.details.get(&cache_key) {
            Some(d) => d.clone(),
            None => {
                let d = Rc::new(self.build_detail(&page, width));
                self.details.insert(cache_key, d.clone());
                d
            }
        };
        if let Some((line, link)) = self.jump.take() {
            self.scroll = detail
                .source_line
                .iter()
                .position(|&l| l >= line)
                .unwrap_or(0);
            self.selected_hit = link.and_then(|link| {
                detail
                    .hits
                    .iter()
                    .position(|h| matches!(h.target, HitTarget::Link(i) if i == link))
            });
        }
        let max = detail.lines.len().saturating_sub(usize::from(height));
        self.scroll = self.scroll.min(max);
        detail
    }

    fn build_detail(&self, page: &Page, width: u16) -> Detail {
        match page {
            Page::Summary => self.summary(),
            Page::Note(rel) => self.note_detail(rel, width),
            Page::Unresolved(target) => self.unresolved_detail(target),
            Page::Ambiguous(key) => self.ambiguous_detail(key),
            Page::Attachment(rel) => self.attachment_detail(rel),
        }
    }

    fn text_detail(&self, title: String, lines: Vec<Line<'static>>, hits: Vec<Hit>) -> Detail {
        let n = lines.len();
        Detail {
            title,
            lines,
            source_line: vec![1; n],
            hits,
        }
    }

    fn summary(&self) -> Detail {
        let notes = self
            .index
            .files
            .iter()
            .filter(|f| f.kind == Kind::Note)
            .count();
        let (mut resolved, mut ambiguous, mut unresolved) = (0, 0, 0);
        for r in self.index.forward.iter().flatten() {
            match r {
                Resolved::File { .. } => resolved += 1,
                Resolved::Ambiguous { .. } => ambiguous += 1,
                Resolved::Unresolved => unresolved += 1,
            }
        }
        let dim = self.theme.dim();
        let lines = vec![
            Line::from(Span::styled(
                self.index.root.display().to_string(),
                Style::new().add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from(format!(
                "{notes} notes, {} attachments",
                self.index.files.len() - notes
            )),
            Line::from(format!(
                "{resolved} links resolved, {ambiguous} ambiguous, {unresolved} unresolved"
            )),
            Line::default(),
            Line::from(Span::styled(
                "Open a note from the tree. ? shows the keys.",
                dim,
            )),
        ];
        self.text_detail(self.root_label.clone(), lines, Vec::new())
    }

    fn note_detail(&self, rel: &str, width: u16) -> Detail {
        let Some(id) = self.index.id(rel) else {
            return self.text_detail(
                rel.to_string(),
                vec![Line::from(format!("deleted: {rel}"))],
                Vec::new(),
            );
        };
        let text = std::fs::read_to_string(self.index.root.join(rel)).unwrap_or_default();
        let Some(parsed) = &self.index.files[id].parsed else {
            return self.text_detail(rel.to_string(), vec![Line::from(text)], Vec::new());
        };
        let r = render::render(
            &text,
            parsed,
            &self.index.forward[id],
            width,
            self.fold_frontmatter,
            self.theme,
        );
        Detail {
            title: rel.to_string(),
            lines: r.lines,
            source_line: r.source_line,
            hits: r
                .hits
                .into_iter()
                .map(|h| Hit {
                    line: h.line,
                    cols: h.cols,
                    target: HitTarget::Link(h.link),
                })
                .collect(),
        }
    }

    fn unresolved_detail(&self, target: &str) -> Detail {
        let want = key(target);
        let want = want.strip_suffix(".md").unwrap_or(&want).to_string();
        let mut lines = vec![
            Line::from(vec![
                Span::raw("No note named "),
                Span::styled(
                    target.to_string(),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::default(),
            Line::from(Span::styled("Referenced from:", self.theme.dim())),
        ];
        let mut hits = Vec::new();
        for (source, file) in self.index.files.iter().enumerate() {
            for (link, state) in self
                .index
                .links(source)
                .iter()
                .zip(&self.index.forward[source])
            {
                let k = key(&link.target);
                if matches!(state, Resolved::Unresolved)
                    && k.strip_suffix(".md").unwrap_or(&k) == want
                {
                    let text = format!("{}:{}", file.rel, link.line);
                    hits.push(Hit {
                        line: lines.len(),
                        cols: 2..2 + text.chars().count() as u16,
                        target: HitTarget::Open {
                            rel: file.rel.clone(),
                            line: link.line,
                        },
                    });
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(text, Style::new().add_modifier(Modifier::UNDERLINED)),
                    ]));
                }
            }
        }
        self.text_detail(format!("unresolved: {target}"), lines, hits)
    }

    fn ambiguous_detail(&self, key: &str) -> Detail {
        let Some(g) = self
            .index
            .unresolved()
            .into_iter()
            .find(|g| g.ambiguous && g.key == key)
        else {
            return self.text_detail(
                key.to_string(),
                vec![Line::from(format!("{key} is no longer ambiguous"))],
                Vec::new(),
            );
        };
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let underline = Style::new().add_modifier(Modifier::UNDERLINED);
        let mut lines = vec![
            Line::from(vec![
                Span::styled(g.shown.clone(), bold),
                Span::raw(format!(" matches {} files", g.candidates.len())),
            ]),
            Line::default(),
            Line::from(Span::styled("Candidates:", self.theme.dim())),
        ];
        let mut hits = Vec::new();
        let mut row = |lines: &mut Vec<Line<'static>>, text: String, rel: String, line: u32| {
            hits.push(Hit {
                line: lines.len(),
                cols: 2..2 + text.chars().count() as u16,
                target: HitTarget::Open { rel, line },
            });
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(text, underline),
            ]));
        };
        for &c in &g.candidates {
            let rel = self.index.files[c].rel.clone();
            row(&mut lines, rel.clone(), rel, 1);
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Referenced from:",
            self.theme.dim(),
        )));
        for s in &g.sources {
            let rel = self.index.files[s.file].rel.clone();
            let goes = s
                .pick
                .map_or(String::new(), |p| format!(" → {}", self.index.files[p].rel));
            row(&mut lines, format!("{rel}:{}{goes}", s.line), rel, s.line);
        }
        self.text_detail(format!("ambiguous: {}", g.shown), lines, hits)
    }

    fn attachment_detail(&self, rel: &str) -> Detail {
        let size = std::fs::metadata(self.index.root.join(rel)).map_or(0, |m| m.len());
        let kind = rel.rsplit_once('.').map_or("file", |(_, ext)| ext);
        let lines = vec![
            Line::from(Span::styled(
                rel.to_string(),
                Style::new().add_modifier(Modifier::BOLD),
            )),
            Line::default(),
            Line::from(format!("{kind}, {size} bytes")),
        ];
        self.text_detail(rel.to_string(), lines, Vec::new())
    }

    // ---- navigation ----

    fn navigate(&mut self, page: Page, jump: Option<(u32, Option<usize>)>) {
        self.history[self.cursor].scroll = self.scroll;
        self.history.truncate(self.cursor + 1);
        self.history.push(Visit { page, scroll: 0 });
        self.cursor += 1;
        self.scroll = 0;
        self.selected_hit = None;
        self.jump = jump;
        if self.mode != Mode::Tree {
            self.rows = None;
            self.list.select(Some(0));
        }
    }

    fn page_exists(&self, page: &Page) -> bool {
        match page {
            Page::Note(rel) | Page::Attachment(rel) => self.index.id(rel).is_some(),
            _ => true,
        }
    }

    fn step_history(&mut self, back: bool) {
        self.history[self.cursor].scroll = self.scroll;
        let mut c = self.cursor;
        loop {
            let next = if back {
                c.checked_sub(1)
            } else {
                (c + 1 < self.history.len()).then_some(c + 1)
            };
            let Some(n) = next else {
                self.status = Some(
                    if back {
                        "start of history"
                    } else {
                        "end of history"
                    }
                    .into(),
                );
                return;
            };
            c = n;
            if self.page_exists(&self.history[c].page) {
                break;
            }
        }
        self.cursor = c;
        self.scroll = self.history[c].scroll;
        self.selected_hit = None;
        self.jump = None;
        if self.mode != Mode::Tree {
            self.rows = None;
        }
    }

    pub fn open_rel(&mut self, rel: &str, line: Option<u32>, link: Option<usize>) {
        let Some(id) = self.index.id(rel) else {
            self.status = Some(format!("not found: {rel}"));
            return;
        };
        let page = if self.index.files[id].kind == Kind::Note {
            Page::Note(rel.to_string())
        } else {
            Page::Attachment(rel.to_string())
        };
        self.navigate(page, line.map(|l| (l, link)));
    }

    /// Follows link `i` of the open note.
    pub fn follow(&mut self, i: usize) {
        let Some(source) = self.open_id() else {
            return;
        };
        let link = self.index.links(source)[i].clone();
        let state = self.index.forward[source][i].clone();
        let Some(target) = state.target() else {
            self.navigate(Page::Unresolved(link.target.clone()), None);
            return;
        };
        let rel = self.index.files[target].rel.clone();
        let line = link
            .fragment
            .as_deref()
            .and_then(|f| match block_fragment(f) {
                Some(block) => self.block_line(&rel, block),
                None => self.index.heading_line(target, f, link.markdown),
            });
        if let Resolved::Ambiguous { others, .. } = &state {
            self.status = Some(format!("ambiguous: {} other candidates", others.len()));
        }
        self.open_rel(&rel, line.or(Some(1)), None);
    }

    fn block_line(&self, rel: &str, block: &str) -> Option<u32> {
        let text = std::fs::read_to_string(self.index.root.join(rel)).ok()?;
        let suffix = format!(" ^{}", block.to_lowercase());
        text.split('\n')
            .position(|l| l.trim_end().to_lowercase().ends_with(&suffix))
            .map(|i| i as u32 + 1)
    }

    fn activate(&mut self, action: RowAction) {
        match action {
            RowAction::Folder(path) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
                self.rows = None;
            }
            RowAction::Open { rel, line, link } => self.open_rel(&rel, line, link),
            RowAction::Follow(i) => self.follow(i),
            RowAction::Tag(key) => {
                if !self.expanded_tags.remove(&key) {
                    self.expanded_tags.insert(key);
                }
                self.rows = None;
            }
            RowAction::Show(page) => self.navigate(page, None),
            RowAction::Nothing => {}
        }
    }

    fn follow_hit(&mut self, hit: &Hit) {
        match hit.target.clone() {
            HitTarget::Link(i) => self.follow(i),
            HitTarget::Open { rel, line } => self.open_rel(&rel, Some(line), None),
        }
    }

    // ---- input ----

    /// The note's text starts two columns in (a border and a space) and one
    /// row down (the title).
    pub fn detail_width(&self) -> u16 {
        self.detail_area.width.saturating_sub(2).max(10)
    }

    fn detail_height(&self) -> usize {
        usize::from(self.detail_area.height.saturating_sub(1)).max(1)
    }

    fn move_list(&mut self, delta: isize) {
        let len = self.rows().len();
        if len == 0 {
            return;
        }
        let cur = self.list.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1);
        self.list.select(Some(next as usize));
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll = (self.scroll as isize + delta).max(0) as usize;
    }

    fn move_by(&mut self, delta: isize) {
        match self.focus {
            Focus::List => self.move_list(delta),
            Focus::Detail => self.scroll_by(delta),
        }
    }

    fn next_link(&mut self, forward: bool) {
        let width = self.detail_width();
        let height = self.detail_height() as u16;
        let detail = self.detail(width, height);
        if detail.hits.is_empty() {
            self.status = Some("no links".into());
            return;
        }
        // Segments of one wrapped link count once.
        let starts: Vec<usize> = (0..detail.hits.len())
            .filter(|&i| i == 0 || !same_target(&detail.hits[i - 1].target, &detail.hits[i].target))
            .collect();
        let cur = self.selected_hit;
        let pick = match (cur, forward) {
            (Some(c), true) => starts.iter().copied().find(|&s| s > c),
            (Some(c), false) => starts.iter().rev().copied().find(|&s| s < c),
            (None, true) => starts
                .iter()
                .copied()
                .find(|&s| detail.hits[s].line >= self.scroll),
            (None, false) => starts.last().copied(),
        };
        let Some(pick) = pick.or_else(|| {
            if forward {
                starts.first().copied()
            } else {
                starts.last().copied()
            }
        }) else {
            return;
        };
        self.selected_hit = Some(pick);
        let line = detail.hits[pick].line;
        let height = self.detail_height();
        if line < self.scroll || line >= self.scroll + height {
            self.scroll = line.saturating_sub(height / 3);
        }
    }

    pub fn key(&mut self, k: KeyEvent) {
        if k.kind != KeyEventKind::Press {
            return;
        }
        self.status = None;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if self.help {
            if matches!(
                k.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.help = false;
            }
            return;
        }
        if self.query_open {
            self.query_key(k);
            return;
        }
        let half = (self.detail_height() / 2).max(1) as isize;
        match k.code {
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('d') if ctrl => self.move_by(half),
            KeyCode::Char('u') if ctrl => self.move_by(-half),
            KeyCode::Char('o') if ctrl => self.step_history(true),
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Esc => self.focus = Focus::List,
            KeyCode::Tab | KeyCode::BackTab => {
                let i = Mode::ALL.iter().position(|m| *m == self.mode).unwrap_or(0);
                let n = Mode::ALL.len();
                let next = if k.code == KeyCode::Tab {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                self.mode = Mode::ALL[next];
                self.rows = None;
                self.list.select(Some(0));
                self.focus = Focus::List;
            }
            KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::List,
            KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Detail,
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(half),
            KeyCode::PageUp => self.move_by(-half),
            KeyCode::Home => self.move_by(isize::MIN / 2),
            KeyCode::End => self.move_by(isize::MAX / 2),
            KeyCode::Char('[') => self.step_history(true),
            KeyCode::Char(']') => self.step_history(false),
            KeyCode::Char('n') => self.next_link(true),
            KeyCode::Char('N') => self.next_link(false),
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.focus = Focus::List;
                self.query_open = true;
                self.rows = None;
            }
            KeyCode::Char('o') => self.edit(),
            KeyCode::Char('y') => self.copy(false),
            KeyCode::Char('Y') => self.copy(true),
            KeyCode::Char('f') => {
                self.fold_frontmatter = !self.fold_frontmatter;
                self.selected_hit = None;
            }
            KeyCode::Enter => self.enter(),
            _ => {}
        }
    }

    fn enter(&mut self) {
        match self.focus {
            Focus::List => {
                let rows = self.rows();
                if let Some(row) = self.list.selected().and_then(|i| rows.get(i)) {
                    self.activate(row.action.clone());
                }
            }
            Focus::Detail => {
                let width = self.detail_width();
                let detail = self.detail(width, self.detail_height() as u16);
                match self.selected_hit.and_then(|i| detail.hits.get(i)) {
                    Some(hit) => self.follow_hit(&hit.clone()),
                    None => self.status = Some("no link selected; n selects one".into()),
                }
            }
        }
    }

    pub fn mouse(&mut self, m: MouseEvent) {
        let at = |r: Rect| {
            m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
        };
        let in_list = at(self.list_area);
        let in_detail = at(self.detail_area);
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown {
                    3
                } else {
                    -3
                };
                if in_list {
                    self.move_list(d);
                } else if in_detail {
                    self.scroll_by(d);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if in_list => {
                self.focus = Focus::List;
                let row = self.list.offset() + usize::from(m.row - self.list_area.y);
                let rows = self.rows();
                if let Some(r) = rows.get(row) {
                    self.list.select(Some(row));
                    self.activate(r.action.clone());
                }
            }
            MouseEventKind::Down(MouseButton::Left) if in_detail => {
                self.focus = Focus::Detail;
                // The first detail row is the title.
                let Some(row) = (m.row - self.detail_area.y).checked_sub(1) else {
                    return;
                };
                let line = self.scroll + usize::from(row);
                let Some(col) = (m.column - self.detail_area.x).checked_sub(2) else {
                    return;
                };
                let width = self.detail_width();
                let detail = self.detail(width, self.detail_height() as u16);
                if let Some(hit) = detail
                    .hits
                    .iter()
                    .find(|h| h.line == line && h.cols.contains(&col))
                {
                    self.follow_hit(&hit.clone());
                }
            }
            _ => {}
        }
    }

    /// A watcher batch: refresh the index and drop everything drawn from it.
    pub fn batch(&mut self, touched: &BTreeSet<String>) {
        match self.index.refresh(touched) {
            Ok(Some(change)) => {
                self.details.clear();
                self.selected_hit = None;
                self.build_tree();
                self.status = Some(format!(
                    "updated: +{} -{} ~{}",
                    change.added.len(),
                    change.removed.len(),
                    change.modified.len()
                ));
            }
            Ok(None) => {}
            Err(e) => self.status = Some(format!("refresh failed: {e}")),
        }
    }
}

fn same_target(a: &HitTarget, b: &HitTarget) -> bool {
    match (a, b) {
        (HitTarget::Link(x), HitTarget::Link(y)) => x == y,
        _ => false,
    }
}

pub const HELP: &[(&str, &str)] = &[
    ("j k, arrows", "move in the list, or scroll the note"),
    ("ctrl-d ctrl-u, pgdn pgup", "half a page"),
    ("home end", "top and bottom"),
    ("enter", "open, fold a folder, or follow the selected link"),
    ("h l", "focus the list or the note"),
    ("n N", "next and previous link in the note"),
    ("[ ], ctrl-o", "back and forward in history"),
    ("tab shift-tab", "list mode"),
    ("f", "fold or unfold frontmatter"),
    ("/", "search; enter or esc leaves the query"),
    ("o", "open the note in the editor"),
    ("y Y", "copy the path, copy a [[wikilink]]"),
    ("?", "this help"),
    ("esc", "close help; focus the list"),
    ("q, ctrl-c", "quit"),
];

/// The largest whole unit of an age in seconds: `42s`, `5m`, `3h`, `2d`,
/// `6w`, `14mo`, `2y`.
pub fn age(secs: u64) -> String {
    const UNITS: [(u64, &str); 6] = [
        (365 * 86_400, "y"),
        (30 * 86_400, "mo"),
        (7 * 86_400, "w"),
        (86_400, "d"),
        (3_600, "h"),
        (60, "m"),
    ];
    for (size, unit) in UNITS {
        if secs >= size {
            return format!("{}{unit}", secs / size);
        }
    }
    format!("{secs}s")
}
