mod common;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use common::{set_mtime, temp_copy, temp_dir};
use knapp::index::{cache_path, Index, CACHE_FORMAT};

fn load(root: &Path, cache: &Path) -> (Index, knapp::index::LoadStats) {
    Index::load(root, &[], Some(cache)).expect("load")
}

fn notes(index: &Index) -> usize {
    index.files.iter().filter(|f| f.parsed.is_some()).count()
}

#[test]
fn second_load_reuses_every_parse() {
    let root = temp_copy("vault-basic", "cache-reuse");
    let cache = temp_dir("cache-reuse-dir").join("c.json");
    let (first, stats) = load(&root, &cache);
    assert_eq!(stats.reused, 0);
    assert!(stats.stale);
    first.save_cache(&cache).expect("save");

    let (second, stats) = load(&root, &cache);
    assert_eq!(stats.reused, notes(&second));
    assert_eq!(stats.parsed, 0);
    assert!(!stats.stale);
    assert_eq!(first.forward, second.forward);
    assert_eq!(
        first.files.iter().map(|f| &f.parsed).collect::<Vec<_>>(),
        second.files.iter().map(|f| &f.parsed).collect::<Vec<_>>()
    );
}

#[test]
fn bad_caches_fall_back_to_parsing() {
    let root = temp_copy("vault-basic", "cache-bad");
    let cache = temp_dir("cache-bad-dir").join("c.json");
    let (index, _) = load(&root, &cache);
    index.save_cache(&cache).expect("save");
    let good = std::fs::read_to_string(&cache).expect("read cache");

    let other_format = good.replacen(
        &format!("\"format\":{CACHE_FORMAT}"),
        &format!("\"format\":{}", CACHE_FORMAT + 1),
        1,
    );
    let other_root = good.replacen(
        &format!("\"root\":\"{}\"", root.display()),
        "\"root\":\"/elsewhere\"",
        1,
    );
    for (what, text) in [
        ("format", other_format),
        ("root", other_root),
        ("corrupt", "{not json".to_string()),
    ] {
        assert_ne!(text, good, "{what}: replacement did not apply");
        std::fs::write(&cache, text).expect("write cache");
        let (index, stats) = load(&root, &cache);
        assert_eq!(stats.reused, 0, "{what}");
        assert_eq!(stats.parsed, notes(&index), "{what}");
    }
}

#[test]
fn same_tick_rewrite_is_seen() {
    let root = temp_copy("vault-basic", "cache-racy");
    let cache = temp_dir("cache-racy-dir").join("c.json");
    // alpha.md is written now, so its mtime is within two seconds of the
    // cache write below.
    let alpha = root.join("alpha.md");
    let text = std::fs::read_to_string(&alpha).expect("read");
    std::fs::write(&alpha, &text).expect("write");
    let mtime = std::fs::metadata(&alpha)
        .expect("meta")
        .modified()
        .expect("mtime");
    let (index, _) = load(&root, &cache);
    index.save_cache(&cache).expect("save");

    // Same size, same mtime: only the two-second rule catches it.
    std::fs::write(&alpha, text.replace("[[index]]", "[[beta]] ")).expect("rewrite");
    assert_eq!(
        std::fs::metadata(&alpha).unwrap().len() as usize,
        text.len()
    );
    set_mtime(&alpha, mtime);

    let (index, stats) = load(&root, &cache);
    assert!(stats.parsed >= 1);
    let id = index.id("alpha.md").unwrap();
    assert_eq!(index.links(id)[0].target, "beta");
}

#[test]
fn deleted_and_added_files_mark_the_cache_stale() {
    let root = temp_copy("vault-basic", "cache-delete");
    let cache = temp_dir("cache-delete-dir").join("c.json");
    let (index, _) = load(&root, &cache);
    index.save_cache(&cache).expect("save");

    std::fs::remove_file(root.join("img.png")).expect("remove");
    let (index, stats) = load(&root, &cache);
    assert!(index.id("img.png").is_none());
    assert!(stats.stale);
    index.save_cache(&cache).expect("save");

    // Delete one attachment and add another: same count, still stale.
    std::fs::rename(root.join("beta.md"), root.join("gamma.md")).expect("rename");
    let (_, stats) = load(&root, &cache);
    assert!(stats.stale);
}

#[test]
fn refresh_reports_changes_and_flips_state() {
    let root = temp_copy("vault-basic", "refresh");
    let (mut index, _) = Index::load(&root, &[], None).expect("load");
    assert_eq!(index.refresh(&BTreeSet::new()).expect("refresh"), None);

    std::fs::create_dir_all(root.join("x")).expect("mkdir");
    std::fs::rename(root.join("alpha.md"), root.join("x/alpha.md")).expect("move");
    let change = index
        .refresh(&BTreeSet::new())
        .expect("refresh")
        .expect("change");
    assert_eq!(change.added, ["x/alpha.md"]);
    assert_eq!(change.removed, ["alpha.md"]);
    let home = index.id("index.md").unwrap();
    let x = index.id("x/alpha.md").unwrap();
    assert_eq!(index.forward[home][1].target(), Some(x));

    std::fs::create_dir_all(root.join("y")).expect("mkdir");
    std::fs::copy(root.join("x/alpha.md"), root.join("y/alpha.md")).expect("copy");
    index
        .refresh(&BTreeSet::new())
        .expect("refresh")
        .expect("change");
    let home = index.id("index.md").unwrap();
    assert!(matches!(
        index.forward[home][1],
        knapp::index::Resolved::Ambiguous { .. }
    ));

    // A touched note is reparsed even when mtime and size are unchanged.
    let deep = root.join("dir/deep.md");
    let text = std::fs::read_to_string(&deep).unwrap();
    let mtime = std::fs::metadata(&deep).unwrap().modified().unwrap();
    std::fs::write(&deep, text.replace("../index.md", "../beta.md ")).unwrap();
    set_mtime(&deep, mtime);
    let change = index
        .refresh(&BTreeSet::from(["dir/deep.md".to_string()]))
        .expect("refresh")
        .expect("change");
    assert_eq!(change.modified, ["dir/deep.md"]);
}

#[test]
fn cli_rebuild_stats_and_unwritable_cache() {
    let root = temp_copy("vault-basic", "cli-cache");
    let base = temp_dir("cli-cache-env");
    let run = |cache_home: &Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_knapp"))
            .args(args)
            .env("XDG_CONFIG_HOME", base.join("config"))
            .env("XDG_CACHE_HOME", cache_home)
            .env_remove("HERDR_PLUGIN_CONFIG_DIR")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .output()
            .expect("run knapp")
    };
    let stat = |out: &std::process::Output, key: &str| -> String {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}\t")).map(str::to_string))
            .unwrap_or_else(|| panic!("no {key} in stats"))
    };
    let root_arg = root.to_str().unwrap();
    let cache_home = base.join("cache");

    let first = run(&cache_home, &["index", "--root", root_arg, "--stats"]);
    assert!(first.status.success());
    assert_eq!(stat(&first, "cache.parsed"), "4");
    assert!(cache_path(&cache_home.join("knapp"), &root).exists());

    let second = run(&cache_home, &["index", "--root", root_arg, "--stats"]);
    assert_eq!(stat(&second, "cache.reused"), "4");
    assert_eq!(stat(&second, "notes"), "4");
    assert_eq!(stat(&second, "attachments"), "1");

    let rebuilt = run(
        &cache_home,
        &["index", "--root", root_arg, "--rebuild", "--stats"],
    );
    assert_eq!(stat(&rebuilt, "cache.reused"), "0");

    // A cache home that is a file: the command still works and warns once.
    let blocker = base.join("not-a-dir");
    std::fs::write(&blocker, "").unwrap();
    let out = run(&blocker, &["index", "--root", root_arg]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "4 notes, 1 attachments, 16 links"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(err.matches("knapp: cache not written").count(), 1, "{err}");
}

#[test]
fn a_prose_only_edit_is_a_modification() {
    let root = temp_copy("vault-basic", "refresh-prose");
    let (mut index, _) = Index::load(&root, &[], None).expect("load");
    // Same links, headings, and tags; only the text differs.
    std::fs::write(root.join("alpha.md"), "# Alpha\n\nFirst paragraph. ^para1\n\n## Second Section\n\nBack to [[index]]. More prose.\n").unwrap();
    let change = index
        .refresh(&BTreeSet::new())
        .expect("refresh")
        .expect("a prose edit is a change");
    assert_eq!(change.modified, ["alpha.md"]);
}
