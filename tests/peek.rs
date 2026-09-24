mod common;

use std::path::PathBuf;

use common::temp_dir;
use knapp::config::Root;
use knapp::peek::{clean_selection, from_url, root_for};

#[test]
fn file_urls() {
    let t = from_url("file:///Users/s/My%20Notes/a.md#Big%20Idea", "mbp.local").unwrap();
    assert_eq!(t.path, PathBuf::from("/Users/s/My Notes/a.md"));
    assert_eq!(t.fragment.as_deref(), Some("Big Idea"));
    // rg and ls write the host; this machine, short or full, and localhost pass.
    for host in ["mbp.local", "MBP", "localhost", ""] {
        let url = format!("file://{host}/tmp/a.md");
        assert!(from_url(&url, "mbp.local").is_ok(), "{url}");
    }
    assert!(from_url("file://other-box/tmp/a.md", "mbp.local")
        .unwrap_err()
        .contains("another machine"));
    assert!(from_url("https://x/a.md", "mbp").is_err());
    assert_eq!(from_url("file:///a.md#", "h").unwrap().fragment, None);
}

#[test]
fn selections() {
    let cases = [
        ("[[alpha]]", ("alpha", None)),
        ("  [[beta#Top|shown]] ", ("beta", Some("Top"))),
        ("![[img.png]]", ("img.png", None)),
        ("`notes/a.md`", ("notes/a.md", None)),
        ("\"dir/deep\"", ("dir/deep", None)),
        ("first line\nsecond", ("first line", None)),
        ("a.md#^blk", ("a.md", Some("^blk"))),
    ];
    for (input, (target, frag)) in cases {
        let (t, f) = clean_selection(input);
        assert_eq!((t.as_str(), f.as_deref()), (target, frag), "{input:?}");
    }
}

#[test]
fn peek_roots() {
    let base = temp_dir("peek-roots");
    std::fs::create_dir_all(base.join("vault/.obsidian")).unwrap();
    std::fs::create_dir_all(base.join("vault/deep/er")).unwrap();
    std::fs::create_dir_all(base.join("repo/.git")).unwrap();
    std::fs::create_dir_all(base.join("repo/docs")).unwrap();
    std::fs::create_dir_all(base.join("loose")).unwrap();
    std::fs::create_dir_all(base.join("configured/sub")).unwrap();
    for f in [
        "vault/deep/er/a.md",
        "repo/docs/b.md",
        "loose/c.md",
        "configured/sub/d.md",
    ] {
        std::fs::write(base.join(f), "x\n").unwrap();
    }
    let roots = [Root {
        name: Some("c".into()),
        path: base.join("configured"),
        send_allow: Vec::new(),
    }];
    let at = |f: &str| root_for(&base.join(f), &roots);
    assert_eq!(at("vault/deep/er/a.md"), base.join("vault"));
    assert_eq!(at("repo/docs/b.md"), base.join("repo"));
    assert_eq!(at("loose/c.md"), base.join("loose"));
    assert_eq!(at("configured/sub/d.md"), base.join("configured"));
}
