//! Walks a root, applies excludes, and splits notes from attachments.

use std::path::Path;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Note,
    Attachment,
}

#[derive(Debug, Clone)]
pub struct Entry {
    /// Root-relative, `/`-separated.
    pub rel: String,
    pub kind: Kind,
    /// Covered by Obsidian's Excluded files: resolvable, never parsed or shown.
    pub excluded: bool,
    pub mtime_ns: u128,
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub entries: Vec<Entry>,
    /// `userIgnoreFilters` entries knapp does not apply (`/regex/`).
    pub skipped_filters: Vec<String>,
}

pub fn scan(root: &Path, exclude: &[String]) -> Result<Scan, String> {
    let meta = std::fs::metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    if !meta.is_dir() {
        return Err(format!("{}: not a directory", root.display()));
    }
    let (ignore, skipped_filters) = obsidian_filters(root);
    let mut entries = Vec::new();
    walk(root, "", exclude, &ignore, &mut entries)?;
    entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(Scan {
        entries,
        skipped_filters,
    })
}

fn walk(
    dir: &Path,
    prefix: &str,
    exclude: &[String],
    ignore: &[String],
    out: &mut Vec<Entry>,
) -> Result<(), String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for item in read {
        let item = item.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let rel = format!("{prefix}{name}");
        let meta = item
            .metadata()
            .map_err(|e| format!("{}: {e}", item.path().display()))?;
        let file_type = meta.file_type();
        let meta = if file_type.is_symlink() {
            match std::fs::metadata(item.path()) {
                Ok(target) if target.is_file() => target,
                _ => continue,
            }
        } else {
            meta
        };
        if meta.is_dir() {
            let dir_rel = format!("{rel}/");
            if excluded_dir(&dir_rel, exclude) {
                continue;
            }
            walk(&item.path(), &dir_rel, exclude, ignore, out)?;
        } else if meta.is_file() {
            if exclude.iter().any(|e| !e.ends_with('/') && *e == rel) {
                continue;
            }
            let kind = if name.to_ascii_lowercase().ends_with(".md") {
                Kind::Note
            } else {
                Kind::Attachment
            };
            let lower = rel.to_lowercase();
            out.push(Entry {
                excluded: ignore.iter().any(|f| lower.starts_with(f)),
                kind,
                mtime_ns: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos()),
                size: meta.len(),
                rel,
            });
        }
    }
    Ok(())
}

fn excluded_dir(dir_rel: &str, exclude: &[String]) -> bool {
    exclude
        .iter()
        .any(|e| e.ends_with('/') && dir_rel.starts_with(e.as_str()))
}

/// Obsidian's Excluded files from `.obsidian/app.json`: path-prefix filters,
/// lowercased, and the `/regex/` filters knapp skips.
fn obsidian_filters(root: &Path) -> (Vec<String>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(root.join(".obsidian").join("app.json")) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (Vec::new(), Vec::new());
    };
    let mut prefixes = Vec::new();
    let mut skipped = Vec::new();
    for filter in json
        .get("userIgnoreFilters")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
    {
        let filter = filter.trim();
        if filter.is_empty() {
            continue;
        }
        if filter.len() > 2 && filter.starts_with('/') && filter.ends_with('/') {
            skipped.push(filter.to_string());
        } else {
            prefixes.push(filter.to_lowercase());
        }
    }
    (prefixes, skipped)
}
