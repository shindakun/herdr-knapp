//! Markdown to styled lines for the detail panel.

use std::collections::HashMap;
use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::index::Resolved;
use crate::parse::{self, LinkKind, Parsed, Value};

pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    /// Source line (1-based) each rendered line starts at, parallel to `lines`.
    pub source_line: Vec<u32>,
    pub hits: Vec<LinkHit>,
}

/// Where a link landed: one entry per rendered line the link touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkHit {
    pub line: usize,
    pub cols: Range<u16>,
    /// Index into `Parsed::links`.
    pub link: usize,
}

/// Styles, with or without color (`NO_COLOR`).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub color: bool,
}

impl Theme {
    pub fn dim(self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }
    pub fn heading(self) -> Style {
        let s = Style::new().add_modifier(Modifier::BOLD);
        if self.color {
            s.fg(Color::Magenta)
        } else {
            s
        }
    }
    pub fn code(self) -> Style {
        if self.color {
            Style::new().fg(Color::Green)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        }
    }
    pub fn link(self, state: &Resolved) -> Style {
        let s = Style::new().add_modifier(Modifier::UNDERLINED);
        match (state, self.color) {
            (Resolved::File { .. }, true) => s.fg(Color::Cyan),
            (Resolved::Ambiguous { .. }, true) => s.fg(Color::Yellow),
            (Resolved::Unresolved, true) => s.fg(Color::Red).add_modifier(Modifier::DIM),
            (Resolved::File { .. }, false) => s,
            (Resolved::Ambiguous { .. }, false) => s.add_modifier(Modifier::BOLD),
            (Resolved::Unresolved, false) => s.add_modifier(Modifier::DIM),
        }
    }
    pub fn callout(self) -> Style {
        let s = Style::new().add_modifier(Modifier::BOLD);
        if self.color {
            s.fg(Color::Blue)
        } else {
            s
        }
    }
}

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];

pub fn is_image(target: &str) -> bool {
    target
        .rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// A run of inline text with one style, from source offset `src`.
#[derive(Clone)]
struct Atom {
    text: String,
    style: Style,
    link: Option<usize>,
    src: usize,
    /// A hard line break, not text.
    brk: bool,
    /// A soft line break, shown as a space.
    soft: bool,
}

enum Container {
    Quote {
        first: bool,
    },
    List {
        next: Option<u64>,
    },
    Item {
        bullet: Option<String>,
        width: usize,
    },
}

struct Table {
    rows: Vec<Vec<Vec<Atom>>>,
    header_rows: usize,
    src: usize,
}

struct Renderer<'a> {
    width: usize,
    theme: Theme,
    line_starts: Vec<usize>,
    out: Rendered,
    stack: Vec<Container>,
    inline: Vec<Atom>,
    inline_src: Option<usize>,
    style: Vec<Style>,
    link: Option<usize>,
    in_image: bool,
    in_metadata: bool,
    /// URL of an external link being rendered, shown after its text.
    external: Option<(String, String, usize)>,
    code: Option<(String, usize)>,
    table: Option<Table>,
    need_blank: bool,
    links: &'a [parse::Link],
    states: &'a [Resolved],
}

pub fn render(
    text: &str,
    parsed: &Parsed,
    states: &[Resolved],
    width: u16,
    fold_frontmatter: bool,
    theme: Theme,
) -> Rendered {
    let width = usize::from(width.max(10));
    let mut r = Renderer {
        width,
        theme,
        line_starts: std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect(),
        out: Rendered {
            lines: Vec::new(),
            source_line: Vec::new(),
            hits: Vec::new(),
        },
        stack: Vec::new(),
        inline: Vec::new(),
        inline_src: None,
        style: vec![Style::new()],
        link: None,
        in_image: false,
        in_metadata: false,
        external: None,
        code: None,
        table: None,
        need_blank: false,
        links: &parsed.links,
        states,
    };
    r.frontmatter(parsed, fold_frontmatter);

    let by_start: HashMap<usize, usize> = parsed
        .links
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind != LinkKind::Property)
        .map(|(i, l)| (l.span.start, i))
        .collect();
    let comments = parse::comments(text);
    let options = parse::OPTIONS | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        r.event(event, range, text, &by_start, &comments);
    }
    r.flush();
    r.out
}

fn in_ranges(ranges: &[Range<usize>], i: usize) -> bool {
    ranges.iter().any(|r| r.contains(&i))
}

impl Renderer<'_> {
    fn line_of(&self, offset: usize) -> u32 {
        self.line_starts.partition_point(|&s| s <= offset) as u32
    }

    fn cur_style(&self) -> Style {
        *self.style.last().expect("base style")
    }

    fn push_style(&mut self, add: Style) {
        let s = self.cur_style().patch(add);
        self.style.push(s);
    }

    fn pop_style(&mut self) {
        if self.style.len() > 1 {
            self.style.pop();
        }
    }

    fn atom(&mut self, text: &str, style: Style, src: usize) {
        if text.is_empty() {
            return;
        }
        self.inline_src.get_or_insert(src);
        self.inline.push(Atom {
            text: text.to_string(),
            style,
            link: self.link,
            src,
            brk: false,
            soft: false,
        });
    }

    fn emit(&mut self, spans: Vec<Span<'static>>, src_line: u32) {
        self.out.lines.push(Line::from(spans));
        self.out.source_line.push(src_line);
    }

    fn blank(&mut self) {
        let line = self.out.source_line.last().copied().unwrap_or(1);
        self.emit(Vec::new(), line);
    }

    /// Starts a block: a blank line first unless this is the first block or
    /// the start of a tight list item.
    fn block_start(&mut self) {
        self.flush();
        if self.need_blank && !self.out.lines.is_empty() {
            self.blank();
        }
        self.need_blank = false;
    }

    fn block_end(&mut self) {
        self.flush();
        self.need_blank = true;
    }

    /// Prefix spans for the first and following lines of the current inline
    /// run. Takes a pending bullet.
    fn prefixes(&mut self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let dim = self.theme.dim();
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for c in &mut self.stack {
            match c {
                Container::Quote { .. } => {
                    first.push(Span::styled("│ ", dim));
                    rest.push(Span::styled("│ ", dim));
                }
                Container::List { .. } => {}
                Container::Item { bullet, width } => {
                    match bullet.take() {
                        Some(b) => first.push(Span::styled(b, dim)),
                        None => first.push(Span::raw(" ".repeat(*width))),
                    }
                    rest.push(Span::raw(" ".repeat(*width)));
                }
            }
        }
        (first, rest)
    }

    fn flush(&mut self) {
        if self.inline.is_empty() {
            self.inline_src = None;
            return;
        }
        let mut atoms = std::mem::take(&mut self.inline);
        let src = self.inline_src.take().unwrap_or(0);
        // A quote's first line `[!type] title` is a callout.
        if let Some(Container::Quote { first }) = self
            .stack
            .iter_mut()
            .rev()
            .find(|c| matches!(c, Container::Quote { .. }))
        {
            if std::mem::replace(first, false) {
                callout(&mut atoms, self.theme);
            }
        }
        let (first, rest) = self.prefixes();
        self.wrap(atoms, first, rest, src);
    }

    fn wrap(
        &mut self,
        atoms: Vec<Atom>,
        first: Vec<Span<'static>>,
        rest: Vec<Span<'static>>,
        src: usize,
    ) {
        let width = self.width;
        let prefix_width = |p: &[Span]| p.iter().map(|s| s.content.width()).sum::<usize>();
        let mut line: Vec<Span<'static>> = first.clone();
        let mut col = prefix_width(&first);
        let mut line_src = src;
        let mut has_text = false;
        let mut pending: Vec<(usize, Range<usize>, usize)> = Vec::new();

        let finish = |this: &mut Self,
                      line: &mut Vec<Span<'static>>,
                      pending: &mut Vec<(usize, Range<usize>, usize)>,
                      line_src: usize| {
            let idx = this.out.lines.len();
            for (link, cols, _) in pending.drain(..) {
                this.out.hits.push(LinkHit {
                    line: idx,
                    cols: cols.start as u16..cols.end as u16,
                    link,
                });
            }
            let src_line = this.line_of(line_src);
            this.emit(std::mem::take(line), src_line);
        };

        for word in words(&atoms) {
            let (parts, space) = match word {
                Word::Text { parts, space } => (parts, space),
                Word::Break(src) => {
                    finish(self, &mut line, &mut pending, line_src);
                    line = rest.clone();
                    col = prefix_width(&rest);
                    has_text = false;
                    line_src = src;
                    continue;
                }
            };
            let w: usize = parts.iter().map(|(s, _)| s.width()).sum();
            if space && !has_text {
                continue;
            }
            if col + w > width && has_text {
                finish(self, &mut line, &mut pending, line_src);
                line = rest.clone();
                col = prefix_width(&rest);
                has_text = false;
                line_src = parts[0].1.src;
                if space {
                    continue;
                }
            }
            for (text, atom) in parts {
                // A word longer than a whole line is broken by characters.
                let mut remaining = text;
                while col + remaining.width() > width && !remaining.is_empty() {
                    let room = width.saturating_sub(col).max(1);
                    let (head, tail) = split_at_width(&remaining, room);
                    self.place(&mut line, &mut pending, &mut col, &head, atom);
                    finish(self, &mut line, &mut pending, line_src);
                    line = rest.clone();
                    col = prefix_width(&rest);
                    line_src = atom.src;
                    remaining = tail;
                }
                if !remaining.is_empty() {
                    self.place(&mut line, &mut pending, &mut col, &remaining, atom);
                    has_text = true;
                }
            }
        }
        if has_text || !line.is_empty() {
            finish(self, &mut line, &mut pending, line_src);
        }
    }

    fn place(
        &self,
        line: &mut Vec<Span<'static>>,
        pending: &mut Vec<(usize, Range<usize>, usize)>,
        col: &mut usize,
        text: &str,
        atom: &Atom,
    ) {
        let w = text.width();
        if let Some(link) = atom.link {
            match pending.last_mut() {
                Some((l, cols, _)) if *l == link && cols.end == *col => cols.end += w,
                _ => pending.push((link, *col..*col + w, 0)),
            }
        }
        line.push(Span::styled(text.to_string(), atom.style));
        *col += w;
    }

    fn frontmatter(&mut self, parsed: &Parsed, folded: bool) {
        if parsed.frontmatter.is_empty() {
            return;
        }
        let dim = self.theme.dim();
        if folded {
            self.emit(
                vec![Span::styled(
                    format!("▸ frontmatter ({} keys)", parsed.frontmatter.len()),
                    dim,
                )],
                1,
            );
            self.need_blank = true;
            return;
        }
        self.emit(vec![Span::styled("▾ frontmatter", dim)], 1);
        let key_width = parsed
            .frontmatter
            .iter()
            .map(|(k, _)| k.width())
            .max()
            .unwrap_or(0);
        // Property links in file order; each list item takes the next unused
        // one written the same way.
        let mut unused: Vec<usize> = parsed
            .links
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind == LinkKind::Property)
            .map(|(i, _)| i)
            .collect();
        for (key, value) in &parsed.frontmatter {
            let items: Vec<String> = match value {
                Value::Str(s) | Value::Date(s) | Value::Raw(s) => vec![s.clone()],
                Value::Bool(b) => vec![b.to_string()],
                Value::List(items) => items.clone(),
            };
            let mut atoms = Vec::new();
            let mut src = 0;
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    atoms.push(plain(", ", Style::new(), src));
                }
                let found = unused.iter().position(|&i| self.links[i].written == *item);
                match found.map(|pos| unused.remove(pos)) {
                    Some(i) => {
                        let l = &self.links[i];
                        src = l.span.start;
                        atoms.push(Atom {
                            text: l.alias.clone().unwrap_or_else(|| item.clone()),
                            style: self.theme.link(&self.states[i]),
                            link: Some(i),
                            src,
                            brk: false,
                            soft: false,
                        });
                    }
                    None => atoms.push(plain(item, Style::new(), src)),
                }
            }
            let pad = " ".repeat(key_width - key.width() + 2);
            let first = vec![Span::styled(format!("  {key}{pad}"), dim)];
            let rest = vec![Span::raw(" ".repeat(key_width + 4))];
            let before = self.out.lines.len();
            self.wrap(atoms, first, rest, src);
            if src == 0 {
                for l in &mut self.out.source_line[before..] {
                    *l = 1;
                }
            }
        }
        self.need_blank = true;
    }

    fn event(
        &mut self,
        event: Event<'_>,
        range: Range<usize>,
        text: &str,
        by_start: &HashMap<usize, usize>,
        comments: &[Range<usize>],
    ) {
        if let Some((code, _)) = &mut self.code {
            match event {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => self.end_code(),
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(Tag::MetadataBlock(_)) => self.in_metadata = true,
            Event::End(TagEnd::MetadataBlock(_)) => self.in_metadata = false,
            _ if self.in_metadata => {}
            Event::Start(Tag::Paragraph) => {
                if !self.in_tight_item_start() {
                    self.block_start();
                } else {
                    self.flush();
                }
            }
            Event::End(TagEnd::Paragraph) => self.block_end(),
            Event::Start(Tag::Heading { level, .. }) => {
                self.block_start();
                let n = level as usize;
                let marks = format!("{} ", "#".repeat(n));
                let dim = self.theme.dim();
                self.atom(&marks, dim, range.start);
                let h = self.theme.heading();
                self.push_style(h);
            }
            Event::End(TagEnd::Heading(_)) => {
                self.pop_style();
                self.block_end();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                self.block_start();
                self.stack.push(Container::Quote { first: true });
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.flush();
                self.stack.pop();
                self.need_blank = true;
            }
            Event::Start(Tag::List(start)) => {
                if !self.in_tight_item_start() {
                    self.block_start();
                } else {
                    self.flush();
                }
                self.stack.push(Container::List { next: start });
            }
            Event::End(TagEnd::List(_)) => {
                self.flush();
                self.stack.pop();
                self.need_blank = true;
            }
            Event::Start(Tag::Item) => {
                self.flush();
                let bullet = match self.stack.last_mut() {
                    Some(Container::List { next: Some(n) }) => {
                        let b = format!("{n}. ");
                        *n += 1;
                        b
                    }
                    _ => "• ".to_string(),
                };
                let width = bullet.width();
                self.stack.push(Container::Item {
                    bullet: Some(bullet),
                    width,
                });
                self.need_blank = false;
            }
            Event::End(TagEnd::Item) => {
                self.flush();
                self.stack.pop();
            }
            Event::TaskListMarker(done) => {
                if let Some(Container::Item { bullet, width }) = self.stack.last_mut() {
                    let b = format!(
                        "{}[{}] ",
                        bullet.as_deref().unwrap_or("• "),
                        if done { "x" } else { " " }
                    );
                    *width = b.width();
                    *bullet = Some(b);
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                self.block_start();
                self.code = Some((String::new(), range.start));
            }
            Event::Start(Tag::Table(_)) => {
                self.block_start();
                self.table = Some(Table {
                    rows: Vec::new(),
                    header_rows: 0,
                    src: range.start,
                });
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = &mut self.table {
                    t.rows.push(Vec::new());
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(t) = &mut self.table {
                    t.header_rows = t.rows.len();
                }
            }
            Event::Start(Tag::TableCell) => {
                self.inline.clear();
            }
            Event::End(TagEnd::TableCell) => {
                let cell = std::mem::take(&mut self.inline);
                self.inline_src = None;
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(cell);
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = self.table.take() {
                    self.end_table(t);
                }
                self.need_blank = true;
            }
            Event::Rule => {
                self.block_start();
                let line = self.line_of(range.start);
                let rule = "─".repeat(self.width.min(60));
                let dim = self.theme.dim();
                self.emit(vec![Span::styled(rule, dim)], line);
                self.need_blank = true;
            }
            Event::Start(Tag::Emphasis) => {
                self.push_style(Style::new().add_modifier(Modifier::ITALIC))
            }
            Event::Start(Tag::Strong) => self.push_style(Style::new().add_modifier(Modifier::BOLD)),
            Event::Start(Tag::Strikethrough) => {
                self.push_style(Style::new().add_modifier(Modifier::CROSSED_OUT))
            }
            Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => {
                self.pop_style()
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                if in_ranges(comments, range.start) {
                    self.push_style(Style::new());
                    return;
                }
                match by_start.get(&range.start) {
                    Some(&i) => {
                        self.link = Some(i);
                        let style = self.theme.link(&self.states[i]);
                        self.push_style(style);
                    }
                    None => {
                        self.push_style(Style::new().add_modifier(Modifier::UNDERLINED));
                        if !matches!(link_type, LinkType::Autolink | LinkType::Email) {
                            self.external =
                                Some((dest_url.to_string(), String::new(), range.start));
                        }
                    }
                }
            }
            Event::End(TagEnd::Link) => {
                self.pop_style();
                self.link = None;
                if let Some((url, shown, src)) = self.external.take() {
                    if url != shown {
                        let dim = self.theme.dim();
                        self.atom(&format!(" <{url}>"), dim, src);
                    }
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                self.in_image = true;
                if in_ranges(comments, range.start) {
                    return;
                }
                let link = by_start.get(&range.start).copied();
                let target = link.map_or(dest_url.to_string(), |i| self.links[i].target.clone());
                self.link = link;
                if is_image(&target) {
                    let name = target.rsplit('/').next().unwrap_or(&target).to_string();
                    let dim = self.theme.dim();
                    self.atom(&format!("[image:\u{a0}{name}]"), dim, range.start);
                } else {
                    let style = link.map_or(Style::new(), |i| self.theme.link(&self.states[i]));
                    let shown = link
                        .and_then(|i| self.links[i].alias.clone())
                        .unwrap_or(target);
                    self.atom(&format!("↳\u{a0}{shown}"), style, range.start);
                }
                self.link = None;
            }
            Event::End(TagEnd::Image) => self.in_image = false,
            Event::Text(t) => {
                if self.in_image {
                    return;
                }
                if let Some((_, shown, _)) = &mut self.external {
                    shown.push_str(&t);
                }
                let style = self.cur_style();
                self.text(&t, range, text, style, comments);
            }
            Event::Code(t) => {
                if in_ranges(comments, range.start) {
                    return;
                }
                let style = self.cur_style().patch(self.theme.code());
                self.atom(&t, style, range.start);
            }
            Event::SoftBreak => {
                let style = self.cur_style();
                self.atom(" ", style, range.start);
                if let Some(a) = self.inline.last_mut() {
                    a.soft = true;
                }
            }
            Event::HardBreak => {
                self.inline.push(Atom {
                    text: String::new(),
                    style: Style::new(),
                    link: None,
                    src: range.start,
                    brk: true,
                    soft: false,
                });
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                if !in_ranges(comments, range.start) {
                    let dim = self.theme.dim();
                    self.atom(t.trim_end(), dim, range.start);
                }
            }
            Event::FootnoteReference(t) => {
                let dim = self.theme.dim();
                self.atom(&format!("[^{t}]"), dim, range.start);
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                let style = self.theme.code();
                self.atom(&t, style, range.start);
            }
            _ => {}
        }
    }

    /// Text outside comments; a text event that crosses a comment keeps only
    /// its visible parts.
    fn text(
        &mut self,
        t: &str,
        range: Range<usize>,
        source: &str,
        style: Style,
        comments: &[Range<usize>],
    ) {
        let overlapping: Vec<&Range<usize>> = comments
            .iter()
            .filter(|c| c.start < range.end && range.start < c.end)
            .collect();
        if overlapping.is_empty() {
            self.atom(t, style, range.start);
            return;
        }
        if source.get(range.clone()) != Some(t) {
            // Escapes or entities changed the text; keep it unless it is
            // entirely inside a comment.
            if !overlapping
                .iter()
                .any(|c| c.start <= range.start && range.end <= c.end)
            {
                self.atom(t, style, range.start);
            }
            return;
        }
        let mut pos = range.start;
        for c in overlapping {
            if c.start > pos {
                self.atom(&source[pos..c.start], style, pos);
            }
            pos = pos.max(c.end);
        }
        if pos < range.end {
            self.atom(&source[pos..range.end], style, pos);
        }
    }

    /// Whether the next block is the first thing in a list item, which gets
    /// no blank line before it.
    fn in_tight_item_start(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(Container::Item {
                bullet: Some(_),
                ..
            })
        ) && self.inline.is_empty()
    }

    fn end_code(&mut self) {
        let Some((code, src)) = self.code.take() else {
            return;
        };
        let style = self.theme.code();
        let (first, rest) = self.prefixes();
        let prefix_w: usize = first.iter().map(|s| s.content.width()).sum();
        let room = self.width.saturating_sub(prefix_w + 2).max(1);
        let start_line = self.line_of(src) + 1;
        for (i, line) in code.trim_end_matches('\n').split('\n').enumerate() {
            let shown = cut(line, room);
            let mut spans = if i == 0 { first.clone() } else { rest.clone() };
            spans.push(Span::raw("  "));
            spans.push(Span::styled(shown, style));
            self.emit(spans, start_line + i as u32);
        }
        self.need_blank = true;
    }

    fn end_table(&mut self, t: Table) {
        let cols = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let cell_text = |cell: &Vec<Atom>| cell.iter().map(|a| a.text.as_str()).collect::<String>();
        let mut widths: Vec<usize> = (0..cols)
            .map(|c| {
                t.rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|cell| cell_text(cell).trim().width())
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        let sep = 3; // " │ "
        let (first, _) = self.prefixes();
        let prefix_w: usize = first.iter().map(|s| s.content.width()).sum();
        let room = self.width.saturating_sub(prefix_w + sep * (cols - 1));
        while widths.iter().sum::<usize>() > room {
            let (i, w) = widths
                .iter()
                .copied()
                .enumerate()
                .max_by_key(|&(_, w)| w)
                .expect("columns");
            if w <= 3 {
                break;
            }
            widths[i] = w - 1;
        }
        let dim = self.theme.dim();
        let src_line = self.line_of(t.src);
        for (r, row) in t.rows.iter().enumerate() {
            let mut spans = first.clone();
            let mut col = prefix_w;
            let idx = self.out.lines.len();
            for (c, w) in widths.iter().enumerate() {
                if c > 0 {
                    spans.push(Span::styled(" │ ", dim));
                    col += sep;
                }
                let empty = Vec::new();
                let cell = row.get(c).unwrap_or(&empty);
                let pieces = trim_cell(cell);
                let fits = pieces.iter().map(|(s, _)| s.width()).sum::<usize>() <= *w;
                let mut used = 0;
                for (text, a) in pieces {
                    let left = *w - used;
                    let shown = if fits || text.width() < left {
                        text
                    } else {
                        cut(&text, left)
                    };
                    let sw = shown.width();
                    if let Some(link) = a.link {
                        self.out.hits.push(LinkHit {
                            line: idx,
                            cols: (col + used) as u16..(col + used + sw) as u16,
                            link,
                        });
                    }
                    let mut style = a.style;
                    if r < t.header_rows {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    spans.push(Span::styled(shown, style));
                    used += sw;
                    if used >= *w {
                        break;
                    }
                }
                if used < *w {
                    spans.push(Span::raw(" ".repeat(*w - used)));
                }
                col += *w;
            }
            // The `|---|` row takes a source line but is not a row.
            let below_header = u32::from(r >= t.header_rows && t.header_rows > 0);
            self.emit(spans, src_line + r as u32 + below_header);
            if r + 1 == t.header_rows {
                let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                let mut spans = first.clone();
                spans.push(Span::styled(rule.join("─┼─"), dim));
                self.emit(spans, src_line + r as u32 + 1);
            }
        }
    }
}

/// A cell's atoms with surrounding whitespace trimmed, as (text, atom).
fn trim_cell(cell: &[Atom]) -> Vec<(String, &Atom)> {
    let atoms: Vec<&Atom> = cell.iter().filter(|a| !a.text.is_empty()).collect();
    let last = atoms.len().saturating_sub(1);
    atoms
        .iter()
        .enumerate()
        .map(|(n, a)| {
            let mut s = a.text.as_str();
            if n == 0 {
                s = s.trim_start();
            }
            if n == last {
                s = s.trim_end();
            }
            (s.to_string(), *a)
        })
        .filter(|(s, _)| !s.is_empty())
        .collect()
}

fn plain(text: &str, style: Style, src: usize) -> Atom {
    Atom {
        text: text.to_string(),
        style,
        link: None,
        src,
        brk: false,
        soft: false,
    }
}

/// Turns a leading `[!type]` into `▌ Type:` and ends the title line at the
/// first soft break. pulldown-cmark may split `[!type]` across text events,
/// so the marker is read across atoms.
fn callout(atoms: &mut Vec<Atom>, theme: Theme) {
    let head: String = atoms
        .iter()
        .take_while(|a| !a.soft && !a.brk)
        .map(|a| a.text.as_str())
        .collect();
    let Some(rest) = head.strip_prefix("[!") else {
        return;
    };
    let Some(close) = rest.find(']') else {
        return;
    };
    let kind = rest[..close].trim_end_matches(['+', '-']);
    let mut consume = close + 3; // `[!`, the kind, `]`
    while consume > 0 {
        let Some(first) = atoms.first_mut() else {
            return;
        };
        if first.text.len() <= consume {
            consume -= first.text.len();
            atoms.remove(0);
        } else {
            first.text = first.text[consume..].to_string();
            consume = 0;
        }
    }
    let mut title: Vec<char> = kind.chars().collect();
    if let Some(c) = title.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    let src = atoms.first().map_or(0, |a| a.src);
    let mut marker = plain(
        &format!("▌ {}:", title.into_iter().collect::<String>()),
        theme.callout(),
        src,
    );
    marker.soft = false;
    atoms.insert(0, marker);
    if let Some(a) = atoms.iter_mut().find(|a| a.soft) {
        a.brk = true;
        a.text.clear();
    }
}

enum Word<'a> {
    /// Touching pieces from one or more atoms; a line never breaks inside.
    Text {
        parts: Vec<(String, &'a Atom)>,
        space: bool,
    },
    /// A hard break, with the source offset of the text after it.
    Break(usize),
}

/// Groups atoms into words and runs of space. Text from different atoms
/// with no space between (a link and the comma after it) is one word.
fn words(atoms: &[Atom]) -> Vec<Word<'_>> {
    let mut out: Vec<Word> = Vec::new();
    for atom in atoms {
        if atom.brk {
            out.push(Word::Break(atom.src + 1));
            continue;
        }
        for piece in split_words(&atom.text) {
            let space = piece.chars().all(is_break);
            match out.last_mut() {
                Some(Word::Text { parts, space: s }) if *s == space && !space => {
                    parts.push((piece.to_string(), atom));
                }
                _ => out.push(Word::Text {
                    parts: vec![(piece.to_string(), atom)],
                    space,
                }),
            }
        }
    }
    out
}

/// Words and the spaces between them, in order. Only spaces and tabs break;
/// a no-break space keeps a placeholder like `[image: x.png]` whole.
fn split_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = None;
    for (i, c) in text.char_indices() {
        let space = is_break(c);
        match in_space {
            Some(s) if s != space => {
                out.push(&text[start..i]);
                start = i;
            }
            _ => {}
        }
        in_space = Some(space);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn is_break(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Splits so the head fits in `room` columns (at least one character).
fn split_at_width(text: &str, room: usize) -> (String, String) {
    let mut w = 0;
    for (i, c) in text.char_indices() {
        let cw = c.width().unwrap_or(0);
        if w + cw > room && i > 0 {
            return (text[..i].to_string(), text[i..].to_string());
        }
        w += cw;
    }
    (text.to_string(), String::new())
}

/// `text` cut to `room` columns, ending in `…` when cut.
pub fn cut(text: &str, room: usize) -> String {
    if text.width() <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let (head, _) = split_at_width(text, room - 1);
    let head = if head.width() > room - 1 {
        String::new()
    } else {
        head
    };
    format!("{head}…")
}
