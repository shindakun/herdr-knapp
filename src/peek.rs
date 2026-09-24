//! `peek-selection` and `peek`: what to show, and from which root.

use std::path::{Path, PathBuf};

use crate::config::Root;

/// Where a note to peek at is, with a `#fragment` to scroll to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub fragment: Option<String>,
}

/// A clicked `file://[host]/path[#fragment]`. The host must be empty,
/// `localhost`, or this machine (full name or its first label).
pub fn from_url(url: &str, hostname: &str) -> Result<Target, String> {
    let rest = url
        .strip_prefix("file://")
        .ok_or_else(|| format!("not a file URL: {url}"))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => return Err(format!("no path in {url}")),
    };
    let host = host.to_ascii_lowercase();
    let me = hostname.trim().to_ascii_lowercase();
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_string();
    let ok = host.is_empty()
        || host == "localhost"
        || host == me
        || (!me.is_empty() && short(&host) == short(&me));
    if !ok {
        return Err(format!("a link to another machine ({host})"));
    }
    let (path, fragment) = match path.split_once('#') {
        Some((p, f)) => (p, Some(crate::parse::percent_decode(f))),
        None => (path, None),
    };
    Ok(Target {
        path: PathBuf::from(crate::parse::percent_decode(path)),
        fragment: fragment.filter(|f| !f.is_empty()),
    })
}

/// Selected text as a link target: the first line, trimmed, with `[[…]]`,
/// quotes, or backticks around it removed and any `|alias` dropped. Returns
/// the target and its `#fragment`.
pub fn clean_selection(text: &str) -> (String, Option<String>) {
    let mut t = text.lines().next().unwrap_or("").trim();
    for (open, close) in [
        ("![[", "]]"),
        ("[[", "]]"),
        ("`", "`"),
        ("\"", "\""),
        ("'", "'"),
    ] {
        if let Some(inner) = t.strip_prefix(open).and_then(|r| r.strip_suffix(close)) {
            t = inner.trim();
        }
    }
    let t = t.split('|').next().unwrap_or(t);
    match t.split_once('#') {
        Some((path, frag)) if !frag.is_empty() => (path.to_string(), Some(frag.to_string())),
        Some((path, _)) => (path.to_string(), None),
        None => (t.to_string(), None),
    }
}

/// Clipboard text worth peeking at: one line, a `[[wikilink]]` or a path to
/// a `.md` file, with trailing punctuation from a loose selection dropped.
/// Anything else (a paragraph, a password, a URL) is `None`.
pub fn note_shaped(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() > 1024 || t.chars().any(char::is_control) {
        return None;
    }
    let t = t.trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
    let wikilink = t.starts_with("[[") || t.starts_with("![[");
    let (target, _) = clean_selection(t);
    (wikilink || target.to_lowercase().ends_with(".md")).then(|| t.to_string())
}

/// The root a peeked note is shown in: the configured root that contains
/// it, else the nearest folder above it with `.obsidian/` or `.git/`, else
/// its own folder.
pub fn root_for(note: &Path, roots: &[Root]) -> PathBuf {
    let note = std::fs::canonicalize(note).unwrap_or_else(|_| note.to_path_buf());
    for r in roots {
        let base = std::fs::canonicalize(&r.path).unwrap_or_else(|_| r.path.clone());
        if note.starts_with(&base) {
            return base;
        }
    }
    let dir = note.parent().unwrap_or(Path::new("/"));
    dir.ancestors()
        .find(|d| d.join(".obsidian").is_dir() || d.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}
