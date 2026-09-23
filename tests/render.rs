mod common;

use std::path::Path;

use knapp::index::Index;
use knapp::parse::parse;
use knapp::render::{cut, render, Theme};
use ratatui::style::Color;

const PLAIN: Theme = Theme { color: false };

fn text_of(r: &knapp::render::Rendered) -> Vec<String> {
    r.lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn render_str(text: &str, width: u16) -> Vec<String> {
    let parsed = parse(text);
    let states = vec![knapp::index::Resolved::Unresolved; parsed.links.len()];
    text_of(&render(text, &parsed, &states, width, true, PLAIN))
}

#[test]
fn blocks() {
    let text = "# Title\n\n- one\n- [ ] task\n  - nested\n\n1. first\n2. second\n\n> quote\n\n> [!warning] Careful\n> body\n\n```\ncode line that is too long for the width\n```\n\n| A | B |\n|---|---|\n| x | 日本 |\n\n---\n\nend %%hidden%% here\n";
    assert_eq!(
        render_str(text, 30),
        [
            "# Title",
            "",
            "• one",
            "• [ ] task",
            "      • nested",
            "",
            "1. first",
            "2. second",
            "",
            "│ quote",
            "",
            "│ ▌ Warning: Careful",
            "│ body",
            "",
            "  code line that is too long …",
            "",
            "A │ B",
            "──┼─────",
            "x │ 日本",
            "",
            "──────────────────────────────",
            "",
            "end  here",
        ]
    );
}

#[test]
fn wraps_at_width_and_keeps_punctuation_with_links() {
    let lines = render_str("See [[alpha]], then more words here to wrap.", 16);
    assert_eq!(lines, ["See alpha, then", "more words here", "to wrap."]);
    for l in render_str(&"word ".repeat(30), 12) {
        assert!(
            unicode_width::UnicodeWidthStr::width(l.as_str()) <= 12,
            "{l}"
        );
    }
}

#[test]
fn hits_point_at_the_right_links() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-basic");
    let (index, _) = Index::load(&root, &[], None).unwrap();
    let id = index.id("index.md").unwrap();
    let text = std::fs::read_to_string(root.join("index.md")).unwrap();
    let parsed = index.files[id].parsed.as_ref().unwrap();
    let r = render(&text, parsed, &index.forward[id], 60, true, PLAIN);
    let rows = text_of(&r);
    for hit in &r.hits {
        let row: Vec<char> = rows[hit.line].chars().collect();
        let shown: String = row[hit.cols.start as usize..hit.cols.end as usize]
            .iter()
            .collect();
        let link = &parsed.links[hit.link];
        let expected = link.alias.clone().unwrap_or_else(|| {
            let inner = link
                .written
                .trim_start_matches('!')
                .trim_start_matches("[[");
            inner.trim_end_matches("]]").to_string()
        });
        // A wrapped link has one hit per line, each showing part of it.
        let placeholder = shown.starts_with("[image:\u{a0}") || shown.starts_with("↳\u{a0}");
        assert!(
            expected.contains(&shown) || placeholder || link.markdown,
            "hit {hit:?} shows {shown:?} for {}",
            link.written
        );
    }
    // Every non-property link in the body has a hit.
    let body_links = parsed
        .links
        .iter()
        .filter(|l| l.kind != knapp::parse::LinkKind::Property)
        .count();
    let mut linked: Vec<usize> = r.hits.iter().map(|h| h.link).collect();
    linked.dedup();
    assert_eq!(linked.len(), body_links);
}

#[test]
fn frontmatter_folded_and_open() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/vault-basic");
    let (index, _) = Index::load(&root, &[], None).unwrap();
    let id = index.id("beta.md").unwrap();
    let text = std::fs::read_to_string(root.join("beta.md")).unwrap();
    let parsed = index.files[id].parsed.as_ref().unwrap();
    let folded = text_of(&render(&text, parsed, &index.forward[id], 60, true, PLAIN));
    assert_eq!(folded[0], "▸ frontmatter (6 keys)");
    let open = render(&text, parsed, &index.forward[id], 60, false, PLAIN);
    let rows = text_of(&open);
    assert_eq!(rows[5], "  owners   ana, [[alpha]]");
    assert_eq!(open.hits.len(), 1);
    assert_eq!(open.hits[0].line, 5);
}

#[test]
fn link_colors_follow_state_and_no_color() {
    let text = "[[here]] [[missing]]";
    let parsed = parse(text);
    let states = vec![
        knapp::index::Resolved::File {
            id: 0,
            fragment: knapp::index::Fragment::None,
        },
        knapp::index::Resolved::Unresolved,
    ];
    let color = render(text, &parsed, &states, 40, true, Theme { color: true });
    let spans = &color.lines[0].spans;
    assert_eq!(spans[0].style.fg, Some(Color::Cyan));
    assert_eq!(spans[2].style.fg, Some(Color::Red));
    let plain = render(text, &parsed, &states, 40, true, PLAIN);
    assert!(plain.lines[0].spans.iter().all(|s| s.style.fg.is_none()));
}

#[test]
fn cut_marks_truncation() {
    assert_eq!(cut("abcdef", 4), "abc…");
    assert_eq!(cut("abc", 4), "abc");
    assert_eq!(cut("日本語", 4), "日…");
}

#[test]
fn image_slots_reserve_rows_instead_of_a_placeholder() {
    let text = "Before\n\n![[pic.png]]\n\nAfter\n";
    let parsed = parse(text);
    let states = vec![knapp::index::Resolved::Unresolved; parsed.links.len()];
    let slots = std::collections::HashMap::from([(0usize, (6u16, 3u16))]);
    let r = knapp::render::render_with_images(text, &parsed, &states, 40, true, PLAIN, &slots);
    let rows = text_of(&r);
    assert_eq!(rows, ["Before", "", "", "", "", "", "After"]);
    assert_eq!(
        r.images,
        [knapp::render::ImageSlot {
            line: 2,
            rows: 3,
            cols: 6,
            link: 0
        }]
    );
    // Without a slot: the placeholder.
    assert!(render_str(text, 40).contains(&"[image:\u{a0}pic.png]".to_string()));
}
