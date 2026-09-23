//! Forward and back link tables, resolution, and the on-disk cache.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::parse::{self, Link, Parsed};
use crate::scan::{self, Entry, Kind};

/// Bump on any change to `Parsed` or to what `parse` extracts.
pub const CACHE_FORMAT: u32 = 1;

/// A cached parse is not trusted when its file changed this close to the
/// cache write: a second write in the same mtime tick leaves mtime alone.
const RACY_NS: u128 = 2_000_000_000;

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

#[derive(Debug, Default, Clone)]
pub struct LoadStats {
    pub reused: usize,
    pub parsed: usize,
    /// The cache on disk no longer matches the index.
    pub stale: bool,
    pub scan_ms: f64,
    pub cache_read_ms: f64,
    pub parse_ms: f64,
    pub resolve_ms: f64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Change {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct CacheFile<P> {
    format: u32,
    root: String,
    written_ns: u128,
    files: BTreeMap<String, CachedFile<P>>,
}

#[derive(Serialize, Deserialize)]
struct CachedFile<P> {
    kind: Kind,
    mtime_ns: u128,
    size: u64,
    parsed: Option<P>,
}

pub struct Index {
    pub root: PathBuf,
    pub files: Vec<File>,
    pub skipped_filters: Vec<String>,
    exclude: Vec<String>,
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

fn needs_parse(e: &Entry) -> bool {
    e.kind == Kind::Note && !e.excluded
}

fn read_and_parse(root: &Path, rel: &str) -> Option<Parsed> {
    let bytes = std::fs::read(root.join(rel)).ok()?;
    String::from_utf8(bytes)
        .ok()
        .map(|text| parse::parse(&text))
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// `<dir>/index/<hex sha256 of the canonical root>.json`
pub fn cache_path(cache_dir: &Path, canonical_root: &Path) -> PathBuf {
    let digest = Sha256::digest(canonical_root.to_string_lossy().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    cache_dir.join("index").join(format!("{hex}.json"))
}

fn read_cache(path: &Path, root: &Path) -> Option<CacheFile<Parsed>> {
    let bytes = std::fs::read(path).ok()?;
    let cache: CacheFile<Parsed> = serde_json::from_slice(&bytes).ok()?;
    (cache.format == CACHE_FORMAT && cache.root == root.to_string_lossy()).then_some(cache)
}

impl Index {
    /// Scans `root`, reusing parses from the cache file at `cache` when
    /// given. Does not write the cache; see `save_cache`.
    pub fn load(
        root: &Path,
        exclude: &[String],
        cache: Option<&Path>,
    ) -> Result<(Self, LoadStats), String> {
        let root = std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let mut stats = LoadStats::default();

        let t = Instant::now();
        let scan = scan::scan(&root, exclude)?;
        stats.scan_ms = ms(t);

        let t = Instant::now();
        let mut cached = cache.and_then(|p| read_cache(p, &root));
        stats.cache_read_ms = ms(t);
        let written_ns = cached.as_ref().map_or(0, |c| c.written_ns);
        let cached_count = cached.as_ref().map_or(0, |c| c.files.len());
        stats.stale = cached.is_none() || cached_count != scan.entries.len();

        let t = Instant::now();
        let mut files = Vec::with_capacity(scan.entries.len());
        for e in scan.entries {
            let hit = cached.as_mut().and_then(|c| c.files.remove(&e.rel));
            let same = hit
                .as_ref()
                .is_some_and(|h| h.kind == e.kind && h.mtime_ns == e.mtime_ns && h.size == e.size);
            let parsed = if !needs_parse(&e) {
                stats.stale |= !same || hit.is_some_and(|h| h.parsed.is_some());
                None
            } else {
                match hit {
                    Some(h)
                        if same
                            && e.mtime_ns != 0
                            && e.mtime_ns + RACY_NS < written_ns
                            && h.parsed.is_some() =>
                    {
                        stats.reused += 1;
                        h.parsed
                    }
                    _ => {
                        stats.parsed += 1;
                        stats.stale = true;
                        read_and_parse(&root, &e.rel)
                    }
                }
            };
            files.push(File {
                rel: e.rel,
                kind: e.kind,
                excluded: e.excluded,
                mtime_ns: e.mtime_ns,
                size: e.size,
                parsed,
            });
        }
        stats.parse_ms = ms(t);

        let t = Instant::now();
        let index = Self::from_files(root, files, scan.skipped_filters, exclude.to_vec());
        stats.resolve_ms = ms(t);
        Ok((index, stats))
    }

    /// Writes the cache with a temp file and a rename, so concurrent
    /// writers never leave a torn file.
    pub fn save_cache(&self, path: &Path) -> Result<(), String> {
        let files = self
            .files
            .iter()
            .map(|f| {
                (
                    f.rel.clone(),
                    CachedFile {
                        kind: f.kind,
                        mtime_ns: f.mtime_ns,
                        size: f.size,
                        parsed: f.parsed.as_ref(),
                    },
                )
            })
            .collect();
        let cache = CacheFile {
            format: CACHE_FORMAT,
            root: self.root.to_string_lossy().into_owned(),
            written_ns: now_ns(),
            files,
        };
        let json = serde_json::to_vec(&cache).map_err(|e| e.to_string())?;
        let dir = path.parent().ok_or("cache path has no directory")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
    }

    /// Sweeps the tree again. Notes named in `touched` are reparsed even when
    /// their mtime and size look unchanged. `None` when nothing changed.
    pub fn refresh(&mut self, touched: &BTreeSet<String>) -> Result<Option<Change>, String> {
        let scan = scan::scan(&self.root, &self.exclude)?;
        let mut old: HashMap<String, File> = std::mem::take(&mut self.files)
            .into_iter()
            .map(|f| (f.rel.clone(), f))
            .collect();
        let mut change = Change::default();
        let mut files = Vec::with_capacity(scan.entries.len());
        for e in scan.entries {
            let prev = old.remove(&e.rel);
            let same = prev.as_ref().is_some_and(|p| {
                p.kind == e.kind
                    && p.mtime_ns == e.mtime_ns
                    && p.size == e.size
                    && p.excluded == e.excluded
            });
            let parsed = match prev {
                Some(p) if same && !(needs_parse(&e) && touched.contains(&e.rel)) => p.parsed,
                Some(p) => {
                    let parsed = needs_parse(&e)
                        .then(|| read_and_parse(&self.root, &e.rel))
                        .flatten();
                    if parsed != p.parsed || p.excluded != e.excluded || p.kind != e.kind {
                        change.modified.push(e.rel.clone());
                    }
                    parsed
                }
                None => {
                    change.added.push(e.rel.clone());
                    needs_parse(&e)
                        .then(|| read_and_parse(&self.root, &e.rel))
                        .flatten()
                }
            };
            files.push(File {
                rel: e.rel,
                kind: e.kind,
                excluded: e.excluded,
                mtime_ns: e.mtime_ns,
                size: e.size,
                parsed,
            });
        }
        change.removed = old.into_keys().collect();
        change.removed.sort();
        self.files = files;
        self.skipped_filters = scan.skipped_filters;
        if change == Change::default() {
            return Ok(None);
        }
        self.build_names();
        self.resolve_all();
        Ok(Some(change))
    }

    pub fn from_files(
        root: PathBuf,
        files: Vec<File>,
        skipped_filters: Vec<String>,
        exclude: Vec<String>,
    ) -> Self {
        let mut index = Self {
            root,
            files,
            skipped_filters,
            exclude,
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
        self.wikilink_from(target, dirname(&self.keys[source]))
    }

    /// `wikilink` for a link written in a note in `folder` (a `key()`ed
    /// folder path, `""` for the root).
    pub fn wikilink_from(&self, target: &str, folder: &str) -> Vec<FileId> {
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
        let mut folder = folder.to_string();
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

    /// The shortest `[[link]]` target that reaches `id` alone from a note at
    /// the root: trailing path segments, without `.md` for notes.
    pub fn shortest_link(&self, id: FileId) -> String {
        let rel = &self.files[id].rel;
        let bare = match self.files[id].kind {
            Kind::Note => rel.strip_suffix(".md").unwrap_or(rel),
            Kind::Attachment => rel,
        };
        let parts: Vec<&str> = bare.split('/').collect();
        (1..=parts.len())
            .map(|n| parts[parts.len() - n..].join("/"))
            .find(|t| self.wikilink_from(t, "") == [id])
            .unwrap_or_else(|| bare.to_string())
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
        if let Some(block) = block_fragment(fragment) {
            let block = block.to_lowercase();
            return parsed.blocks.iter().any(|b| b.to_lowercase() == block);
        }
        self.heading_line(id, fragment, markdown).is_some()
    }

    /// The line of the heading a fragment names, following Obsidian's
    /// nested `#a#b` rule. `None` for block fragments and misses.
    pub fn heading_line(&self, id: FileId, fragment: &str, markdown: bool) -> Option<u32> {
        let parsed = self.files[id].parsed.as_ref()?;
        let parts: Vec<&str> = fragment.split('#').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || block_fragment(fragment).is_some() {
            return None;
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
                    return Some(h.line);
                }
            }
        }
        None
    }
}

/// `^id` when the fragment is a single block reference.
pub fn block_fragment(fragment: &str) -> Option<&str> {
    let parts: Vec<&str> = fragment.split('#').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [only] => only.strip_prefix('^'),
        _ => None,
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
