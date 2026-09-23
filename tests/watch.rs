mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use common::temp_copy;
use knapp::index::{Change, Index};
use knapp::watch::{watch, Watch};

/// Waits for batches until one makes `refresh` report a change.
fn next_change(w: &Watch, index: &mut Index) -> Change {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let batch = w
            .batches
            .recv_timeout(left)
            .expect("no watcher batch within 5 s");
        if let Some(change) = index.refresh(&batch).expect("refresh") {
            return change;
        }
    }
}

/// Drains batches for `wait` and returns whether any made a change.
fn quiet(w: &Watch, index: &mut Index, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    while let Ok(batch) = w
        .batches
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        if index.refresh(&batch).expect("refresh").is_some() {
            return false;
        }
    }
    true
}

fn start(root: &Path, ignore: Option<&Path>) -> (Watch, Index) {
    let (mut index, _) = Index::load(root, &[], None).expect("load");
    let w = watch(root, ignore).expect("watch");
    // FSEvents may replay events from before the watch; let them settle.
    quiet(&w, &mut index, Duration::from_millis(600));
    (w, index)
}

#[test]
fn create_edit_rename_delete() {
    let root = temp_copy("vault-basic", "watch");
    let (w, mut index) = start(&root, None);

    std::fs::write(root.join("new.md"), "[[alpha]]\n").unwrap();
    let c = next_change(&w, &mut index);
    assert_eq!(c.added, ["new.md"]);
    let alpha = index.id("alpha.md").unwrap();
    assert!(index.back[alpha]
        .iter()
        .any(|&(s, _)| index.files[s].rel == "new.md"));

    std::fs::write(root.join("new.md"), "[[beta]]\n").unwrap();
    let c = next_change(&w, &mut index);
    assert_eq!(c.modified, ["new.md"]);

    std::fs::rename(root.join("new.md"), root.join("renamed.md")).unwrap();
    let c = next_change(&w, &mut index);
    assert_eq!(
        (c.added.as_slice(), c.removed.as_slice()),
        (&["renamed.md".to_string()][..], &["new.md".to_string()][..])
    );

    std::fs::remove_file(root.join("alpha.md")).unwrap();
    let c = next_change(&w, &mut index);
    assert_eq!(c.removed, ["alpha.md"]);
    let home = index.id("index.md").unwrap();
    assert_eq!(index.forward[home][1].target(), None);
    std::fs::remove_dir_all(root).ok();
}

fn batches_for(w: &Watch, wait: Duration) -> Vec<BTreeSet<String>> {
    std::iter::from_fn(|| w.batches.recv_timeout(wait).ok()).collect()
}

#[test]
fn dot_folders_and_the_cache_dir_are_ignored() {
    let root = temp_copy("vault-basic", "watch-ignore");
    let cache = root.join("cache-here");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(root.join(".obsidian")).unwrap();

    let (w, _index) = start(&root, Some(&cache));
    std::fs::write(root.join(".obsidian/workspace.json"), "{}").unwrap();
    std::fs::write(cache.join("c.json"), "{}").unwrap();
    let batches = batches_for(&w, Duration::from_millis(800));
    assert!(
        batches.is_empty(),
        "ignored paths made a batch: {batches:?}"
    );
    drop(w);

    // Control: without the cache-dir ignore, the same write is seen.
    let (w, _index) = start(&root, None);
    std::fs::write(cache.join("c.json"), "{\"x\": 1}").unwrap();
    let batches = batches_for(&w, Duration::from_millis(800));
    assert!(
        batches.iter().any(|b| b.contains("cache-here/c.json")),
        "control write not seen: {batches:?}"
    );
    std::fs::remove_dir_all(root).ok();
}
