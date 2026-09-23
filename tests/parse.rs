use knapp::parse::{parse, LinkKind, Value};

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!("{}/fixtures/{path}", env!("CARGO_MANIFEST_DIR")))
        .expect("read fixture")
}

fn tags(text: &str) -> Vec<String> {
    parse(text).tags.into_iter().map(|t| t.name).collect()
}

#[test]
fn wikilink_parts_and_spans() {
    let text = "See [[dir/Note#Sec|shown]] and ![[img.png]].\n[[n#^blk]] [[#Top]]";
    let links = parse(text).links;
    assert_eq!(links.len(), 4);

    let l = &links[0];
    assert_eq!(l.kind, LinkKind::Link);
    assert!(!l.markdown);
    assert_eq!(l.span, 4..26);
    assert_eq!(&text[l.span.clone()], "[[dir/Note#Sec|shown]]");
    assert_eq!(l.written, "[[dir/Note#Sec|shown]]");
    assert_eq!(l.target, "dir/Note");
    assert_eq!(l.fragment.as_deref(), Some("Sec"));
    assert_eq!(l.alias.as_deref(), Some("shown"));
    assert_eq!(l.line, 1);

    assert_eq!(links[1].kind, LinkKind::Embed);
    assert_eq!(links[1].target, "img.png");
    assert_eq!(&text[links[1].span.clone()], "![[img.png]]");

    assert_eq!(links[2].target, "n");
    assert_eq!(links[2].fragment.as_deref(), Some("^blk"));
    assert_eq!(links[2].line, 2);

    assert_eq!(links[3].target, "");
    assert_eq!(links[3].fragment.as_deref(), Some("Top"));
}

#[test]
fn markdown_links_decode_and_skip_schemes() {
    let text =
        "[a](my%20note.md#Deep%20Notes) [b](https://x.com/a.md) [c](mailto:a@b.c) [d](<sp ace.md>)";
    let links = parse(text).links;
    let targets: Vec<(&str, Option<&str>)> = links
        .iter()
        .map(|l| (l.target.as_str(), l.fragment.as_deref()))
        .collect();
    assert_eq!(
        targets,
        [("my note.md", Some("Deep Notes")), ("sp ace.md", None)]
    );
    assert!(links.iter().all(|l| l.markdown));
}

#[test]
fn table_escaped_pipe_strips_backslash() {
    let text = "| a | b |\n|---|---|\n| [[target\\|shown]] | x |\n";
    let links = parse(text).links;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "target");
    assert_eq!(links[0].alias.as_deref(), Some("shown"));
    assert_eq!(links[0].written, "[[target\\|shown]]");
}

#[test]
fn syntax_fixture_skips_code_and_comments() {
    let parsed = parse(&fixture("vault-syntax/syntax.md"));
    let targets: Vec<&str> = parsed.links.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(
        targets,
        [
            "real",
            "after-comment",
            "table-target",
            "after-code-percent"
        ]
    );
    let names: Vec<&str> = parsed.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["real-tag"]);
}

#[test]
fn comments_keep_offsets() {
    let text = "%%[[hidden]]%% [[shown]]";
    let links = parse(text).links;
    assert_eq!(links.len(), 1);
    assert_eq!(&text[links[0].span.clone()], "[[shown]]");
}

#[test]
fn tag_rules() {
    assert_eq!(
        tags("#a #nested/b x#no `#code` #123 #1a (#paren) \\#esc\n# Heading #in-heading"),
        ["a", "nested/b", "1a", "in-heading"]
    );
}

#[test]
fn vault_basic_tags_blocks_and_frontmatter() {
    let index = parse(&fixture("vault-basic/index.md"));
    let names: Vec<&str> = index.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["home", "meta/index", "inline-tag", "nested/child"]);
    assert_eq!(index.links[0].kind, LinkKind::Property);
    assert_eq!(index.links[0].written, "[[dir/deep]]");
    assert_eq!(index.links[0].line, 3);

    let alpha = parse(&fixture("vault-basic/alpha.md"));
    assert_eq!(alpha.blocks, ["para1"]);
    assert!(alpha.tags.is_empty());

    let beta = parse(&fixture("vault-basic/beta.md"));
    let names: Vec<&str> = beta.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["heading-tag"]);
    let fm: Vec<(&str, &Value)> = beta
        .frontmatter
        .iter()
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    assert_eq!(
        fm,
        [
            ("title", &Value::Str("Beta".into())),
            ("draft", &Value::Bool(false)),
            ("created", &Value::Date("2026-09-22".into())),
            ("aliases", &Value::List(vec!["b".into(), "bee".into()])),
            (
                "owners",
                &Value::List(vec!["ana".into(), "[[alpha]]".into()])
            ),
            ("weird", &Value::Raw("{a: 1}".into())),
        ]
    );
    let heads: Vec<(u8, &str)> = beta
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str()))
        .collect();
    assert_eq!(
        heads,
        [(1, "Top"), (2, "Inner"), (2, "Tagged #heading-tag")]
    );
    let link = &beta.links[0];
    assert_eq!(link.kind, LinkKind::Property);
    assert_eq!(link.target, "alpha");
    assert_eq!(link.line, 8);
    assert_eq!(
        &fixture("vault-basic/beta.md")[link.span.clone()],
        "[[alpha]]"
    );
}

#[test]
fn frontmatter_tags_as_string_and_nested_map() {
    let parsed = parse("---\ntags: one, #two three\nmeta:\n  nested: x\n---\nbody\n");
    let names: Vec<&str> = parsed.tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["one", "two", "three"]);
    assert_eq!(
        parsed.frontmatter[1],
        ("meta".into(), Value::Raw("  nested: x".into()))
    );
}
