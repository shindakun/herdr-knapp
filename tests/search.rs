mod common;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use common::temp_copy;
use knapp::index::Index;
use knapp::scan::Kind;
use knapp::search::{find_rg, parse_rg_match, search, Match};

fn notes(root: &Path, exclude: &[String]) -> Vec<String> {
    let (index, _) = Index::load(root, exclude, None).unwrap();
    index
        .files
        .iter()
        .filter(|f| f.kind == Kind::Note && !f.excluded)
        .map(|f| f.rel.clone())
        .collect()
}

fn run(root: &Path, query: &str, exclude: &[String], rg: Option<&Path>) -> Vec<Match> {
    let mut out = Vec::new();
    let notes = notes(root, exclude);
    search(
        root,
        query,
        exclude,
        &notes,
        rg,
        &AtomicBool::new(false),
        |m| {
            out.push(m);
            true
        },
    )
    .unwrap();
    // The app drops anything outside the index; do the same here.
    out.retain(|m| notes.contains(&m.rel));
    out.sort_by(|a, b| (&a.rel, a.line).cmp(&(&b.rel, b.line)));
    out
}

fn summary(ms: &[Match]) -> Vec<String> {
    ms.iter()
        .map(|m| {
            let marked: Vec<&str> = m.ranges.iter().map(|r| &m.text[r.clone()]).collect();
            format!("{}:{} {:?}", m.rel, m.line, marked)
        })
        .collect()
}

#[test]
fn builtin_smart_case_and_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-syntax");
    assert_eq!(
        summary(&run(&root, "in-code", &[], None)),
        ["syntax.md:5 [\"in-code\"]"]
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-basic");
    let lower = run(&root, "alpha", &[], None);
    let upper = run(&root, "Alpha", &[], None);
    assert!(lower.len() > upper.len(), "{lower:?} {upper:?}");
    assert!(upper
        .iter()
        .all(|m| m.ranges.iter().all(|r| &m.text[r.clone()] == "Alpha")));
}

#[test]
fn ripgrep_matches_builtin_and_respects_the_index() {
    let Some(rg) = find_rg() else {
        eprintln!("rg not on PATH; skipped");
        return;
    };
    let root = temp_copy("vault-basic", "search");
    // Hidden, excluded, and gitignored notes that all contain the query.
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::create_dir_all(root.join("skip")).unwrap();
    std::fs::write(root.join(".hidden/h.md"), "alpha hidden\n").unwrap();
    std::fs::write(root.join("skip/s.md"), "alpha excluded\n").unwrap();
    std::fs::write(root.join("ignored.md"), "alpha gitignored\n").unwrap();
    std::fs::write(root.join(".gitignore"), "ignored.md\n").unwrap();
    let exclude = vec!["skip/".to_string()];

    for query in ["alpha", "Alpha", "Deep Notes", "#nested"] {
        let built = run(&root, query, &exclude, None);
        let fast = run(&root, query, &exclude, Some(&rg));
        assert_eq!(summary(&fast), summary(&built), "query {query:?}");
    }
    let all = summary(&run(&root, "alpha", &exclude, Some(&rg)));
    // .gitignore does not hide a note the tree shows; dot folders and
    // excludes are never listed.
    assert!(all.iter().any(|l| l.starts_with("ignored.md:1")), "{all:?}");
    assert!(
        !all.iter()
            .any(|l| l.contains("hidden") || l.starts_with("skip/")),
        "{all:?}"
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn rg_json_records() {
    let m = parse_rg_match(r#"{"type":"match","data":{"path":{"text":"./a/b.md"},"lines":{"text":"hello World\n"},"line_number":3,"absolute_offset":0,"submatches":[{"match":{"text":"World"},"start":6,"end":11}]}}"#).unwrap();
    assert_eq!(
        (m.rel.as_str(), m.line, m.text.as_str()),
        ("a/b.md", 3, "hello World")
    );
    assert_eq!(m.ranges, [std::ops::Range { start: 6, end: 11 }]);
    assert!(parse_rg_match(r#"{"type":"begin","data":{"path":{"text":"a.md"}}}"#).is_none());
    assert!(parse_rg_match(r#"{"type":"match","data":{"path":{"bytes":"/w=="},"lines":{"text":"x"},"line_number":1,"submatches":[]}}"#).is_none());
}
