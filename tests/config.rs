use std::path::Path;

use knapp::config::Config;

#[test]
fn top_level_key_after_a_root_is_an_error() {
    let err = Config::parse("[[root]]\nname = \"n\"\npath = \".\"\nexclude = [\"x/\"]\n")
        .expect_err("exclude inside [[root]] must fail");
    assert!(err.contains("exclude"), "{err}");
}

#[test]
fn defaults_and_fields() {
    let c = Config::parse(
        "exclude = [\"node_modules/\"]\n\n[[root]]\nname = \"docs\"\npath = \"docs\"\nsend_allow = [\"docs/\"]\n",
    )
    .expect("parse");
    assert_eq!(c.exclude, ["node_modules/"]);
    assert_eq!(c.graph_hops, 2);
    assert_eq!(c.send_max_bytes, 65536);
    let roots = c.roots(Path::new("/work"));
    assert_eq!(roots[0].path, Path::new("/work/docs"));
    assert_eq!(roots[0].send_allow, ["docs/"]);
}

#[test]
fn pick_root_by_name_containment_and_fallback() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let c = Config::parse(&format!(
        "[[root]]\nname = \"basic\"\npath = \"{}/fixtures/vault-basic\"\n",
        repo.display()
    ))
    .expect("parse");
    let by_name = c.pick_root(Some("basic"), repo, repo).expect("pick");
    assert_eq!(by_name.name.as_deref(), Some("basic"));

    let inside = repo.join("fixtures/vault-basic/dir");
    let contained = c.pick_root(None, &inside, repo).expect("pick");
    assert_eq!(contained.name.as_deref(), Some("basic"));

    let fallback = c.pick_root(None, &repo.join("docs"), repo).expect("pick");
    assert_eq!(fallback.name, None);
    assert_eq!(fallback.path, repo);
    assert!(fallback.send_allow.is_empty());

    let by_path = c
        .pick_root(Some("fixtures/vault-broken"), repo, repo)
        .expect("pick");
    assert_eq!(by_path.path, repo.join("fixtures/vault-broken"));
}
