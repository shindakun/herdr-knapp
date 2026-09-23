//! Forward and back link tables, resolution, and the on-disk cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::parse::{self, Link, Parsed};
use crate::scan::{self, Kind};

pub type FileId = usize;

#[derive(Debug, Clone)]
pub struct File {
    pub rel: String,
    pub kind: Kind,
    pub excluded: bool,
    pub mtime_ns: u128,
    pub size: u64,
    /// `None` for attachments, excluded files, and notes that are not UTF-8.
    pub parsed: Option<Parsed>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fragment {
    None,
    Found,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    File {
        id: FileId,
        fragment: Fragment,
    },
    Ambiguous {
        pick: FileId,
        others: Vec<FileId>,
        fragment: Fragment,
    },
    Unresolved,
}

impl Resolved {
    /// Where following the link goes: the file, or Obsidian's pick.
    pub fn target(&self) -> Option<FileId> {
        match self {
            Resolved::File { id, .. } => Some(*id),
            Resolved::Ambiguous { pick, .. } => Some(*pick),
            Resolved::Unresolved => None,
        }
    }
}

pub struct Index {
    pub root: PathBuf,
    pub files: Vec<File>,
    pub skipped_filters: Vec<String>,
    keys: Vec<String>,
    by_path: HashMap<String, FileId>,
    by_name: HashMap<String, Vec<FileId>>,
    /// Per file, parallel to `Parsed::links`; empty for unparsed files.
    pub forward: Vec<Vec<Resolved>>,
    /// Per file: (source note, link position).
    pub back: Vec<Vec<(FileId, usize)>>,
}

pub fn key(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

fn basename(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn dirname(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

impl Index {
    pub fn load(root: &Path, exclude: &[String]) -> Result<Self, String> {
        let root = std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let scan = scan::scan(&root, exclude)?;
        let files = scan
            .entries
            .into_iter()
            .map(|e| {
                let parsed = (e.kind == Kind::Note && !e.excluded)
                    .then(|| std::fs::read(root.join(&e.rel)).ok())
                    .flatten()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .map(|text| parse::parse(&text));
                File {
                    rel: e.rel,
                    kind: e.kind,
                    excluded: e.excluded,
                    mtime_ns: e.mtime_ns,
                    size: e.size,
                    parsed,
                }
            })
            .collect();
        Ok(Self::from_files(root, files, scan.skipped_filters))
    }

    pub fn from_files(root: PathBuf, files: Vec<File>, skipped_filters: Vec<String>) -> Self {
        let mut index = Self {
            root,
            files,
            skipped_filters,
            keys: Vec::new(),
            by_path: HashMap::new(),
            by_name: HashMap::new(),
            forward: Vec::new(),
            back: Vec::new(),
        };
        index.build_names();
        index.resolve_all();
        index
    }

    fn build_names(&mut self) {
        self.keys = self.files.iter().map(|f| key(&f.rel)).collect();
        self.by_path.clear();
        self.by_name.clear();
        for (id, k) in self.keys.iter().enumerate() {
            self.by_path.insert(k.clone(), id);
            self.by_name
                .entry(basename(k).to_string())
                .or_default()
                .push(id);
        }
    }

    fn resolve_all(&mut self) {
        self.forward = (0..self.files.len())
            .map(|id| match &self.files[id].parsed {
                Some(p) => p.links.iter().map(|l| self.resolve(l, id)).collect(),
                None => Vec::new(),
            })
            .collect();
        self.back = vec![Vec::new(); self.files.len()];
        for (source, links) in self.forward.iter().enumerate() {
            if self.files[source].excluded {
                continue;
            }
            for (pos, r) in links.iter().enumerate() {
                if let Some(target) = r.target().filter(|&t| t != source) {
                    self.back[target].push((source, pos));
                }
            }
        }
    }

    pub fn id(&self, rel: &str) -> Option<FileId> {
        self.by_path.get(&key(rel)).copied()
    }

    pub fn links(&self, id: FileId) -> &[Link] {
        self.files[id]
            .parsed
            .as_ref()
            .map_or(&[][..], |p| p.links.as_slice())
    }

    pub fn resolve(&self, link: &Link, source: FileId) -> Resolved {
        let found = if link.markdown {
            self.markdown(&link.target, source)
        } else {
            self.wikilink(&link.target, source)
        };
        let Some((&first, rest)) = found.split_first() else {
            return Resolved::Unresolved;
        };
        let fragment = match &link.fragment {
            None => Fragment::None,
            Some(f) if self.fragment_found(first, f, link.markdown) => Fragment::Found,
            Some(_) => Fragment::Missing,
        };
        if rest.is_empty() {
            Resolved::File {
                id: first,
                fragment,
            }
        } else {
            Resolved::Ambiguous {
                pick: first,
                others: rest.to_vec(),
                fragment,
            }
        }
    }

    /// Obsidian 1.14.2's `getLinkpathDest`: the candidates in its order.
    /// Equal lengths are ordered by path.
    pub fn wikilink(&self, target: &str, source: FileId) -> Vec<FileId> {
        if target.is_empty() {
            return vec![source];
        }
        let mut link = key(target);
        let mut name = basename(&link).to_string();
        let mut candidates = if name.contains('.') {
            self.by_name.get(&name)
        } else {
            None
        };
        if candidates.is_none() {
            link = key(&format!("{target}.md"));
            name = basename(&link).to_string();
            candidates = self.by_name.get(&name);
        }
        let Some(candidates) = candidates else {
            return Vec::new();
        };
        if name == link && candidates.len() == 1 {
            return candidates.clone();
        }
        let mut folder = dirname(&self.keys[source]).to_string();
        let exact = |path: &str| candidates.iter().copied().find(|&c| self.keys[c] == path);

        if link.starts_with("./") || link.starts_with("../") {
            if link.starts_with("./../") {
                link = link[2..].to_string();
            }
            // Obsidian updates the folder here too, and the same-folder
            // ordering below uses the updated value.
            let rest = if let Some(rest) = link.strip_prefix("./") {
                rest.to_string()
            } else {
                let mut rest = link.as_str();
                while let Some(r) = rest.strip_prefix("../") {
                    rest = r;
                    folder = dirname(&folder).to_string();
                }
                rest.to_string()
            };
            if !folder.is_empty() {
                folder.push('/');
            }
            link = format!("{folder}{rest}");
            if let Some(c) = exact(&link) {
                return vec![c];
            }
        }
        let link = link.strip_prefix('/').unwrap_or(&link).to_string();
        if let Some(c) = exact(&link) {
            return vec![c];
        }
        if target.starts_with('/') {
            return Vec::new();
        }
        // Obsidian's plain string tests: `ab/x.md` ends with `b/x.md`, and
        // `notes2/x.md` starts with `notes`.
        let by_len = |a: &FileId, b: &FileId| {
            let (pa, pb) = (&self.files[*a].rel, &self.files[*b].rel);
            pa.len().cmp(&pb.len()).then_with(|| pa.cmp(pb))
        };
        let kept: Vec<FileId> = candidates
            .iter()
            .copied()
            .filter(|&c| self.keys[c].ends_with(&link))
            .collect();
        let (mut near, mut far): (Vec<FileId>, Vec<FileId>) = kept
            .into_iter()
            .partition(|&c| self.keys[c].starts_with(&folder));
        near.sort_by(by_len);
        far.sort_by(by_len);
        near.extend(far);
        near
    }

    /// A Markdown link: relative to the source note first. A path that climbs
    /// above the root is unresolved; one that does not exist falls back to the
    /// wikilink rules.
    pub fn markdown(&self, target: &str, source: FileId) -> Vec<FileId> {
        if target.is_empty() {
            return vec![source];
        }
        if !target.starts_with('/') {
            let Some(joined) = normalize(&join(dirname(&self.files[source].rel), target)) else {
                return Vec::new();
            };
            if let Some(&id) = self.by_path.get(&key(&joined)) {
                return vec![id];
            }
        }
        self.wikilink(target, source)
    }

    /// Obsidian's `resolveSubpath`, plus GitHub slugs for Markdown links.
    fn fragment_found(&self, id: FileId, fragment: &str, markdown: bool) -> bool {
        let Some(parsed) = &self.files[id].parsed else {
            return false;
        };
        let parts: Vec<&str> = fragment.split('#').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return false;
        }
        if let [only] = parts.as_slice() {
            if let Some(id) = only.strip_prefix('^') {
                let id = id.to_lowercase();
                return parsed.blocks.iter().any(|b| b.to_lowercase() == id);
            }
        }
        let mut matched = 0;
        let mut depth = 0u8;
        for h in &parsed.headings {
            let part = parts[matched];
            let hit = heading_norm(&h.text) == heading_norm(part)
                || (markdown && github_slug(&h.text) == part.to_lowercase());
            if h.level > depth && hit {
                matched += 1;
                depth = h.level;
                if matched == parts.len() {
                    return true;
                }
            }
        }
        false
    }
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

/// Resolves `.` and `..` without touching the filesystem. `None` when the
/// path climbs above the root.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

fn heading_norm(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| {
            if "!\"#$%&()*+,.:;<=>?@^`{|}~/[]\\\r\n".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    replaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn github_slug(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|&c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}
