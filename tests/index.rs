use std::path::{Path, PathBuf};

use knapp::index::{Fragment, Index, Resolved};

fn temp_copy(fixture: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("knapp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(fixture),
        &dir,
    );
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).expect("copy");
        }
    }
}

fn resolved_to(index: &Index, source: &str, pos: usize) -> Resolved {
    index.forward[index.id(source).expect("source")][pos].clone()
}

#[test]
fn self_links_are_not_backlinks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-broken");
    let index = Index::load(&root, &[]).expect("load");
    let me = index.id("self.md").expect("self.md");
    assert!(index.back[me].is_empty());
    assert_eq!(
        resolved_to(&index, "self.md", 1),
        Resolved::File {
            id: me,
            fragment: Fragment::Found
        }
    );
}

#[test]
fn moving_and_adding_files_changes_state() {
    let root = temp_copy("vault-basic", "move");
    std::fs::create_dir_all(root.join("x")).expect("mkdir");
    std::fs::rename(root.join("alpha.md"), root.join("x/alpha.md")).expect("move");
    let index = Index::load(&root, &[]).expect("load");
    let x = index.id("x/alpha.md").expect("x/alpha.md");
    assert!(matches!(resolved_to(&index, "index.md", 1), Resolved::File { id, .. } if id == x));

    std::fs::create_dir_all(root.join("y")).expect("mkdir");
    std::fs::copy(root.join("x/alpha.md"), root.join("y/alpha.md")).expect("copy");
    let index = Index::load(&root, &[]).expect("load");
    let (x, y) = (
        index.id("x/alpha.md").unwrap(),
        index.id("y/alpha.md").unwrap(),
    );
    assert!(matches!(
        resolved_to(&index, "index.md", 1),
        Resolved::Ambiguous { pick, others, .. } if pick == x && others == [y]
    ));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn obsidian_excluded_files_resolve_but_do_not_link() {
    let root = temp_copy("vault-basic", "excluded");
    std::fs::create_dir_all(root.join(".obsidian")).expect("mkdir");
    std::fs::write(
        root.join(".obsidian/app.json"),
        r#"{"userIgnoreFilters": ["Alpha.md", "/^dir/"]}"#,
    )
    .expect("write app.json");
    let index = Index::load(&root, &[]).expect("load");
    let alpha = index.id("alpha.md").expect("alpha.md");
    assert!(index.files[alpha].excluded);
    assert!(index.files[alpha].parsed.is_none());
    assert_eq!(index.skipped_filters, ["/^dir/"]);
    // index.md still links to it, but alpha's own link to index.md is gone.
    assert!(matches!(resolved_to(&index, "index.md", 1), Resolved::File { id, .. } if id == alpha));
    let home = index.id("index.md").unwrap();
    assert!(index.back[home].iter().all(|&(s, _)| s != alpha));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn nfd_file_names_match_nfc_links() {
    let root = std::env::temp_dir().join(format!("knapp-nfc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    // "Café" with a combining acute accent (NFD).
    std::fs::write(root.join("Cafe\u{301}.md"), "# Café\n").expect("write");
    std::fs::write(root.join("home.md"), "[[caf\u{e9}]]\n").expect("write");
    let index = Index::load(&root, &[]).expect("load");
    assert!(matches!(
        resolved_to(&index, "home.md", 0),
        Resolved::File { .. }
    ));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn exclude_config_skips_files_and_dirs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-basic");
    let index = Index::load(&root, &["dir/".into(), "img.png".into()]).expect("load");
    assert!(index.id("dir/deep.md").is_none());
    assert!(index.id("img.png").is_none());
    assert_eq!(resolved_to(&index, "index.md", 0), Resolved::Unresolved);
}
