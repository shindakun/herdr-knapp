mod common;

use std::path::Path;

use common::temp_copy;

use knapp::index::{Fragment, Index, Resolved};

fn resolved_to(index: &Index, source: &str, pos: usize) -> Resolved {
    index.forward[index.id(source).expect("source")][pos].clone()
}

#[test]
fn self_links_are_not_backlinks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-broken");
    let index = Index::load(&root, &[], None).expect("load").0;
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
    let index = Index::load(&root, &[], None).expect("load").0;
    let x = index.id("x/alpha.md").expect("x/alpha.md");
    assert!(matches!(resolved_to(&index, "index.md", 1), Resolved::File { id, .. } if id == x));

    std::fs::create_dir_all(root.join("y")).expect("mkdir");
    std::fs::copy(root.join("x/alpha.md"), root.join("y/alpha.md")).expect("copy");
    let index = Index::load(&root, &[], None).expect("load").0;
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
    let index = Index::load(&root, &[], None).expect("load").0;
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
    let index = Index::load(&root, &[], None).expect("load").0;
    assert!(matches!(
        resolved_to(&index, "home.md", 0),
        Resolved::File { .. }
    ));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn exclude_config_skips_files_and_dirs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-basic");
    let index = Index::load(&root, &["dir/".into(), "img.png".into()], None)
        .expect("load")
        .0;
    assert!(index.id("dir/deep.md").is_none());
    assert!(index.id("img.png").is_none());
    assert_eq!(resolved_to(&index, "index.md", 0), Resolved::Unresolved);
}

fn tag_vault(name: &str, notes: &[(&str, &str)]) -> std::path::PathBuf {
    let root = common::temp_dir(name);
    for (rel, text) in notes {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

#[test]
fn tags_fold_case_keep_the_most_used_spelling_and_count_notes() {
    let root = tag_vault(
        "tags",
        &[
            ("a.md", "#Project/Alpha #project/beta\n"),
            ("b.md", "#project/alpha #project/alpha\n"),
            ("c.md", "#project/ALPHA #solo\n"),
        ],
    );
    let (index, _) = Index::load(&root, &[], None).unwrap();
    let tags = index.tags();
    let shown: Vec<(&str, usize)> = tags.iter().map(|t| (t.shown.as_str(), t.count)).collect();
    // `project` is written `project` three times and `Project` once.
    assert_eq!(shown, [("project", 3), ("solo", 1)]);
    let kids: Vec<(&str, usize, usize)> = tags[0]
        .children
        .iter()
        .map(|t| (t.name(), t.count, t.notes.len()))
        .collect();
    // alpha: a.md, b.md (twice, one note), c.md; spelled `alpha` twice.
    assert_eq!(kids, [("alpha", 3, 3), ("beta", 1, 1)]);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn orphans_skip_self_links_excluded_notes_and_attachments() {
    let root = tag_vault(
        "orphans",
        &[
            ("a.md", "[[a]] only myself\n"),
            ("b.md", "[[c]]\n"),
            ("c.md", "linked from b\n"),
            ("hidden.md", "nobody links here\n"),
            ("pic.png", "not a note"),
            (
                ".obsidian/app.json",
                r#"{"userIgnoreFilters": ["hidden.md"]}"#,
            ),
        ],
    );
    let (index, _) = Index::load(&root, &[], None).unwrap();
    let orphans: Vec<&str> = index
        .orphans()
        .into_iter()
        .map(|id| index.files[id].rel.as_str())
        .collect();
    assert_eq!(orphans, ["a.md", "b.md"]);
    std::fs::remove_dir_all(root).ok();
}
