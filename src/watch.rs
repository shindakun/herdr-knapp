//! Filesystem notifier and incremental reindex.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode, EventKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

const QUIET: Duration = Duration::from_millis(150);
const MAX_WAIT: Duration = Duration::from_secs(1);

/// Watches a root. Each item on `batches` is the set of root-relative paths
/// events named since the last batch; it may be empty when events only
/// named folders. Dropping the `Watch` stops the watcher and its thread.
pub struct Watch {
    _watcher: RecommendedWatcher,
    pub batches: Receiver<BTreeSet<String>>,
}

impl Watch {
    /// Blocks for the next batch; `None` once the watcher has stopped. A
    /// thread that reads batches should call this, not `batches` directly:
    /// a closure that names only the `batches` field captures only that
    /// field, and the watcher is dropped.
    pub fn next_batch(&self) -> Option<BTreeSet<String>> {
        self.batches.recv().ok()
    }
}

pub fn watch(root: &Path, ignore_dir: Option<&Path>) -> Result<Watch, String> {
    let root = std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let ignore_dir =
        ignore_dir.map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf()));
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(raw_tx).map_err(|e| format!("watch: {e}"))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("watch {}: {e}", root.display()))?;
    let (tx, batches) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(first) = raw_rx.recv() {
            let mut batch = BTreeSet::new();
            let mut any = add(&root, ignore_dir.as_deref(), first, &mut batch);
            let start = Instant::now();
            loop {
                match raw_rx.recv_timeout(QUIET) {
                    Ok(event) => {
                        any |= add(&root, ignore_dir.as_deref(), event, &mut batch);
                        if start.elapsed() >= MAX_WAIT {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if any && tx.send(batch).is_err() {
                return;
            }
        }
    });
    Ok(Watch {
        _watcher: watcher,
        batches,
    })
}

/// Adds an event's paths to `batch`. Returns whether any path counts: one
/// inside the root, outside dot folders and the cache directory.
fn add(
    root: &Path,
    ignore_dir: Option<&Path>,
    event: notify::Result<notify::Event>,
    batch: &mut BTreeSet<String>,
) -> bool {
    let Ok(event) = event else {
        // An error (such as a dropped event queue) still means the tree may
        // have changed; the sweep will find out.
        return true;
    };
    // inotify reports opens and reads (notify watches IN_OPEN), and the
    // sweep opens every folder: counting those would make each refresh
    // trigger the next. A close after writing is a change; other access is
    // not.
    if let EventKind::Access(kind) = event.kind {
        if kind != AccessKind::Close(AccessMode::Write) {
            return false;
        }
    }
    let mut any = false;
    for path in event.paths {
        if let Some(rel) = relevant(root, ignore_dir, &path) {
            any = true;
            if !rel.is_empty() {
                batch.insert(rel);
            }
        }
    }
    any
}

fn relevant(root: &Path, ignore_dir: Option<&Path>, path: &Path) -> Option<String> {
    if ignore_dir.is_some_and(|d| path.starts_with(d)) {
        return None;
    }
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return None;
    }
    Some(parts.join("/"))
}
