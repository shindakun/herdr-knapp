use std::path::Path;

use knapp::graph::{canvas, dot, layout, local, spec, tree, Mark};
use knapp::index::Index;

fn load(fixture: &str) -> Index {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(fixture);
    Index::load(&root, &[], None).unwrap().0
}

#[test]
fn hops_limit_the_walk() {
    let index = load("vault-broken");
    let hub = index.id("hub.md").unwrap();
    let one = local(&index, hub, 1, None);
    let names: Vec<&str> = one
        .nodes
        .iter()
        .map(|n| index.files[n.id].rel.as_str())
        .collect();
    assert_eq!(names, ["hub.md", "loop-a.md", "orphan.md", "ref.md"]);
    assert_eq!(one.nodes[2].mark, Mark::In);
    let two = local(&index, hub, 2, None);
    assert_eq!(two.nodes.len(), 6); // plus loop-b and loop-c
    assert_eq!(local(&index, hub, 0, None).nodes.len(), 1);
}

#[test]
fn the_cap_cuts_the_farthest_hop_first() {
    let index = load("vault-broken");
    let hub = index.id("hub.md").unwrap();
    let capped = local(&index, hub, 2, Some(4));
    assert_eq!(capped.left_out, 2);
    assert!(capped.nodes.iter().all(|n| n.hop <= 1));
    let lines = tree(&index, &capped);
    assert_eq!(lines.last().unwrap(), "(2 more not shown)");
}

#[test]
fn dot_escapes_and_lists_edges() {
    let index = load("vault-broken");
    let a = index.id("loop-a.md").unwrap();
    let text = dot(&index, &local(&index, a, 1, None));
    assert!(text.starts_with("digraph knapp {\n"));
    assert!(text.contains("  \"loop-a.md\" -> \"loop-b.md\";\n"));
    assert!(text.contains("  \"loop-c.md\" -> \"loop-a.md\";\n"));
    assert!(text.ends_with("}\n"));
}

#[test]
fn layout_is_stable_and_cells_are_unique() {
    let index = load("vault-basic");
    let root = index.id("index.md").unwrap();
    let l = local(&index, root, 2, None);
    let first = layout(&index, &l, 60, 16);
    let again = layout(&index, &l, 60, 16);
    assert_eq!(first, again);
    let mut cells = first.clone();
    cells.sort();
    cells.dedup();
    assert_eq!(cells.len(), first.len(), "{first:?}");
    assert!(first.iter().all(|&(x, y)| x < 60 && y < 16));
    // The root sits in the middle.
    let (x, y) = first[0];
    assert!(
        (28..=32).contains(&x) && (7..=9).contains(&y),
        "{:?}",
        first[0]
    );
}

#[test]
fn canvas_is_a_png_of_cells_times_cell_size() {
    let index = load("vault-basic");
    let root = index.id("index.md").unwrap();
    let l = local(&index, root, 2, None);
    let cells = layout(&index, &l, 40, 12);
    let s = spec(&index, &l, &cells, 40, 12);
    assert_eq!(s.dots.len(), l.nodes.len());
    assert!(s.dots[0].2, "the root is marked");
    let png = canvas(&s, (8, 16)).unwrap();
    let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert_eq!((pm.width(), pm.height()), (320, 192));
    assert_eq!(knapp::herdr::png_size(&png), Some((320, 192)));
    // Transparent away from the dots and edges.
    assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0);
}

#[test]
fn png_size_reads_ihdr() {
    let png = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/vault-basic/img.png"
    ))
    .unwrap();
    assert_eq!(knapp::herdr::png_size(&png), Some((2, 2)));
    assert_eq!(knapp::herdr::png_size(b"not a png at all, no"), None);
}
