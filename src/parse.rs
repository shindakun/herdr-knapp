//! Extracts wikilinks, embeds, Markdown links, tags, frontmatter, and headings from one note.

use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag as MdTag, TagEnd};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    Link,
    Embed,
    Property,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
    /// `[text](path)` form; resolved relative to the note first.
    pub markdown: bool,
    /// Source text of the link.
    pub written: String,
    /// Path part, decoded, table-escape backslash stripped.
    pub target: String,
    /// After the first `#`, without it.
    pub fragment: Option<String>,
    pub alias: Option<String>,
    pub span: Range<usize>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    Str(String),
    List(Vec<String>),
    Date(String),
    Bool(bool),
    Raw(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parsed {
    pub links: Vec<Link>,
    pub tags: Vec<Tag>,
    pub headings: Vec<Heading>,
    pub blocks: Vec<String>,
    pub frontmatter: Vec<(String, Value)>,
}

pub const OPTIONS: Options = Options::ENABLE_WIKILINKS
    .union(Options::ENABLE_TABLES)
    .union(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

pub fn parse(text: &str) -> Parsed {
    let lines = LineIndex::new(text);
    let code = code_ranges(text);
    let comments = comment_ranges(text, &code);

    let mut out = Parsed::default();
    let mut frontmatter: Option<Range<usize>> = None;
    let mut in_link = 0usize;
    let mut in_code_block = false;
    let mut in_metadata = false;
    let mut heading: Option<(u8, usize, String)> = None;

    for (event, range) in Parser::new_ext(text, OPTIONS).into_offset_iter() {
        match event {
            Event::Start(MdTag::MetadataBlock(_)) => {
                in_metadata = true;
                frontmatter = Some(range);
            }
            Event::End(TagEnd::MetadataBlock(_)) => in_metadata = false,
            Event::Start(MdTag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Start(MdTag::Heading { level, .. }) if !in_ranges(&comments, range.start) => {
                heading = Some((level_number(level), range.start, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, start, text)) = heading.take() {
                    out.headings.push(Heading {
                        level,
                        text: text.trim().to_string(),
                        line: lines.line(start),
                    });
                }
            }
            Event::Start(MdTag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                in_link += 1;
                if !in_ranges(&comments, range.start) {
                    out.links
                        .extend(link_from(text, &lines, link_type, &dest_url, range, false));
                }
            }
            Event::Start(MdTag::Image {
                link_type,
                dest_url,
                ..
            }) => {
                in_link += 1;
                if !in_ranges(&comments, range.start) {
                    out.links
                        .extend(link_from(text, &lines, link_type, &dest_url, range, true));
                }
            }
            Event::End(TagEnd::Link | TagEnd::Image) => in_link = in_link.saturating_sub(1),
            Event::Text(t) => {
                if let Some((_, _, h)) = heading.as_mut() {
                    h.push_str(&t);
                }
                if in_link == 0 && !in_code_block && !in_metadata {
                    scan_tags(text, range, &comments, &lines, &mut out.tags);
                }
            }
            Event::Code(t) => {
                if let Some((_, _, h)) = heading.as_mut() {
                    h.push_str(&t);
                }
            }
            _ => {}
        }
    }

    if let Some(range) = frontmatter {
        parse_frontmatter(text, range, &lines, &mut out);
    }
    out.blocks = block_ids(text, &code, &comments);
    out.links.sort_by_key(|l| l.span.start);
    out.tags.sort_by_key(|t| t.line);
    out
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

struct LineIndex(Vec<usize>);

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self(starts)
    }

    fn line(&self, offset: usize) -> u32 {
        self.0.partition_point(|&s| s <= offset) as u32
    }
}

/// `%%comment%%` byte ranges in `text`, outside code.
pub fn comments(text: &str) -> Vec<Range<usize>> {
    comment_ranges(text, &code_ranges(text))
}

fn code_ranges(text: &str) -> Vec<Range<usize>> {
    Parser::new_ext(text, OPTIONS)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Code(_) | Event::Start(MdTag::CodeBlock(_)) => Some(range),
            _ => None,
        })
        .collect()
}

fn in_ranges(ranges: &[Range<usize>], i: usize) -> bool {
    ranges.iter().any(|r| r.contains(&i))
}

/// `%%comment%%` ranges outside code. An unpaired `%%` runs to the end of
/// the file. The text is parsed as written; anything starting inside a
/// comment is dropped. Blanking comments out would change block structure: a
/// line that starts with one would become an indented code block.
fn comment_ranges(text: &str, code: &[Range<usize>]) -> Vec<Range<usize>> {
    let marks: Vec<usize> = text
        .match_indices("%%")
        .map(|(i, _)| i)
        .filter(|&i| !in_ranges(code, i))
        .collect();
    let mut ranges = Vec::new();
    let mut last_end = 0;
    for (i, &open) in marks.iter().enumerate() {
        if open < last_end {
            continue;
        }
        let close = marks[i + 1..].iter().copied().find(|&c| c >= open + 2);
        let end = close.map_or(text.len(), |c| c + 2);
        ranges.push(open..end);
        last_end = end;
    }
    ranges
}

fn link_from(
    text: &str,
    lines: &LineIndex,
    link_type: LinkType,
    dest: &str,
    span: Range<usize>,
    image: bool,
) -> Option<Link> {
    let written = text.get(span.clone())?.to_string();
    let kind = if image {
        LinkKind::Embed
    } else {
        LinkKind::Link
    };
    match link_type {
        LinkType::WikiLink { has_pothole } => {
            let dest = dest.strip_suffix('\\').unwrap_or(dest);
            let (target, fragment) = split_fragment(dest);
            let alias = has_pothole.then(|| wiki_alias(&written)).flatten();
            Some(Link {
                kind,
                markdown: false,
                target,
                fragment,
                alias,
                line: lines.line(span.start),
                span,
                written,
            })
        }
        LinkType::Inline
        | LinkType::Reference
        | LinkType::ReferenceUnknown
        | LinkType::Collapsed
        | LinkType::CollapsedUnknown
        | LinkType::Shortcut
        | LinkType::ShortcutUnknown => {
            if has_scheme(dest) {
                return None;
            }
            let dest = dest
                .strip_prefix('<')
                .and_then(|d| d.strip_suffix('>'))
                .unwrap_or(dest);
            let (target, fragment) = split_fragment(&percent_decode(dest));
            Some(Link {
                kind,
                markdown: true,
                target,
                fragment,
                alias: None,
                line: lines.line(span.start),
                span,
                written,
            })
        }
        _ => None,
    }
}

fn split_fragment(dest: &str) -> (String, Option<String>) {
    match dest.split_once('#') {
        Some((target, fragment)) => (target.to_string(), Some(fragment.to_string())),
        None => (dest.to_string(), None),
    }
}

fn wiki_alias(written: &str) -> Option<String> {
    let inner = written.trim_start_matches('!').strip_prefix("[[")?;
    let inner = inner.strip_suffix("]]")?;
    let (_, alias) = inner.split_once('|')?;
    Some(alias.to_string())
}

/// `https:`, `mailto:`, and any other URI scheme: not a note link.
fn has_scheme(dest: &str) -> bool {
    let Some((scheme, _)) = dest.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && scheme.len() > 1
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

fn scan_tags(
    text: &str,
    range: Range<usize>,
    comments: &[Range<usize>],
    lines: &LineIndex,
    tags: &mut Vec<Tag>,
) {
    let Some(segment) = text.get(range.clone()) else {
        return;
    };
    for (offset, _) in segment.match_indices('#') {
        let at = range.start + offset;
        if in_ranges(comments, at) {
            continue;
        }
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let name: String = text[at + 1..]
            .chars()
            .take_while(|&c| is_tag_char(c))
            .collect();
        if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        tags.push(Tag {
            name,
            line: lines.line(at),
        });
    }
}

/// `^id` at the end of a line outside code and comments, as written.
fn block_ids(text: &str, code: &[Range<usize>], comments: &[Range<usize>]) -> Vec<String> {
    let mut ids = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let line_start = start;
        start += line.len();
        let trimmed = line.trim_end();
        if in_ranges(code, line_start)
            || in_ranges(comments, line_start + trimmed.len().saturating_sub(1))
        {
            continue;
        }
        let Some((before, last)) = trimmed.rsplit_once(' ') else {
            continue;
        };
        let Some(id) = last.strip_prefix('^') else {
            continue;
        };
        if !before.trim().is_empty()
            && !id.is_empty()
            && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            ids.push(id.to_string());
        }
    }
    ids
}

/// One scalar value with its byte range in the file, quotes removed.
struct Item {
    text: String,
    span: Range<usize>,
}

fn unquote(raw: &str, start: usize) -> Item {
    let trimmed_start = raw.len() - raw.trim_start().len();
    let t = raw.trim();
    let start = start + trimmed_start;
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            return Item {
                text: t[1..t.len() - 1].to_string(),
                span: start + 1..start + t.len() - 1,
            };
        }
    }
    Item {
        text: t.to_string(),
        span: start..start + t.len(),
    }
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn scalar(item: &Item) -> Value {
    match item.text.as_str() {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        s if is_date(s) => Value::Date(s.to_string()),
        s => Value::Str(s.to_string()),
    }
}

/// The YAML subset notes use: `key: value`, `key: [a, b]`, and `key:`
/// followed by `- a` lines. Anything else is kept as raw text.
fn parse_frontmatter(text: &str, range: Range<usize>, lines: &LineIndex, out: &mut Parsed) {
    let block = &text[range.clone()];
    let mut rows: Vec<(usize, &str)> = Vec::new();
    let mut offset = range.start;
    for line in block.split_inclusive('\n') {
        rows.push((offset, line.trim_end_matches(['\n', '\r'])));
        offset += line.len();
    }
    // Drop the opening and closing fences.
    let body = if rows.len() >= 2 {
        &rows[1..rows.len() - 1]
    } else {
        &[][..]
    };

    let mut i = 0;
    while i < body.len() {
        let (start, line) = body[i];
        i += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') || line.starts_with(' ') {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim().to_string();
        let rest = &line[colon + 1..];
        let rest_start = start + colon + 1;
        let rest_trimmed = rest.trim();

        let mut items: Vec<Item> = Vec::new();
        let value = if rest_trimmed.is_empty() {
            let mut raw_lines = Vec::new();
            let mut list = true;
            while i < body.len() && (body[i].1.starts_with(' ') || body[i].1.starts_with('-')) {
                let (s, l) = body[i];
                raw_lines.push(l);
                let lead = l.len() - l.trim_start().len();
                match l.trim_start().strip_prefix("- ") {
                    Some(v) => items.push(unquote(v, s + lead + 2)),
                    None => list = false,
                }
                i += 1;
            }
            if raw_lines.is_empty() {
                Value::Str(String::new())
            } else if list {
                Value::List(items.iter().map(|it| it.text.clone()).collect())
            } else {
                items.clear();
                Value::Raw(raw_lines.join("\n"))
            }
        } else if rest_trimmed.starts_with('[') && rest_trimmed.ends_with(']') {
            let open = rest_start + rest.find('[').unwrap_or(0) + 1;
            let inner = &rest_trimmed[1..rest_trimmed.len() - 1];
            let mut pos = open;
            for part in inner.split(',') {
                if !part.trim().is_empty() {
                    items.push(unquote(part, pos));
                }
                pos += part.len() + 1;
            }
            Value::List(items.iter().map(|it| it.text.clone()).collect())
        } else if rest_trimmed.starts_with('{') {
            Value::Raw(rest_trimmed.to_string())
        } else {
            let item = unquote(rest, rest_start);
            let value = scalar(&item);
            items.push(item);
            value
        };

        if key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag") {
            let names: Vec<(String, usize)> = match &value {
                Value::List(_) => items
                    .iter()
                    .map(|it| (it.text.clone(), it.span.start))
                    .collect(),
                Value::Str(s) => s
                    .split([',', ' '])
                    .filter(|t| !t.is_empty())
                    .map(|t| (t.to_string(), rest_start))
                    .collect(),
                _ => Vec::new(),
            };
            for (name, at) in names {
                let name = name.trim_start_matches('#').to_string();
                if !name.is_empty() {
                    out.tags.push(Tag {
                        name,
                        line: lines.line(at),
                    });
                }
            }
        }

        for item in &items {
            if let Some(link) = property_link(item, lines) {
                out.links.push(link);
            }
        }
        out.frontmatter.push((key, value));
    }
}

fn property_link(item: &Item, lines: &LineIndex) -> Option<Link> {
    let t = &item.text;
    let line = lines.line(item.span.start);
    if let Some(inner) = t.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        let (path, alias) = match inner.split_once('|') {
            Some((p, a)) => (p, Some(a.to_string())),
            None => (inner, None),
        };
        let (target, fragment) = split_fragment(path);
        return Some(Link {
            kind: LinkKind::Property,
            markdown: false,
            written: t.clone(),
            target,
            fragment,
            alias,
            span: item.span.clone(),
            line,
        });
    }
    if t.starts_with('[') && t.ends_with(')') {
        let (_, dest) = t.split_once("](")?;
        let dest = &dest[..dest.len() - 1];
        if has_scheme(dest) {
            return None;
        }
        let (target, fragment) = split_fragment(&percent_decode(dest));
        return Some(Link {
            kind: LinkKind::Property,
            markdown: true,
            written: t.clone(),
            target,
            fragment,
            alias: None,
            span: item.span.clone(),
            line,
        });
    }
    None
}
