mod common;

use std::collections::BTreeSet;
use std::time::Instant;

use common::{age, temp_dir};
use knapp::index::Index;

const NOTES: usize = 5000;

/// A small fixed-seed generator, so every run builds the same vault.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

fn generate(root: &std::path::Path) {
    let mut rng = Lcg(1);
    let filler = "Lorem ipsum dolor sit amet. ".repeat(40);
    for i in 0..NOTES {
        let dir = root.join(format!("dir{:02}/sub{}", i % 50, i % 7));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let links: Vec<String> = (0..20)
            .map(|_| format!("[[note-{:04}]]", rng.below(NOTES)))
            .collect();
        let body = format!(
            "---\ntags: [t{}, area/x{}]\ncreated: 2026-01-01\n---\n# note-{i:04}\n\n## Section {i}\n\n{filler}\n\n{filler}\n\n{filler}\n\n{}\n\n#tag{} [[missing-{}]] ^blk{i}\n",
            i % 30,
            i % 5,
            links.join(" "),
            i % 100,
            i % 200,
        );
        std::fs::write(dir.join(format!("note-{i:04}.md")), body).expect("write");
    }
}

/// Run with `cargo test --release -- --ignored`. Asserts the targets in
/// docs/PLAN.md for 5,000 notes.
#[test]
#[ignore]
fn five_thousand_notes_meet_targets() {
    let root = temp_dir("perf");
    generate(&root);
    age(&root);
    let cache = temp_dir("perf-cache").join("c.json");

    let t = Instant::now();
    let (index, stats) = Index::load(&root, &[], Some(&cache)).expect("cold load");
    let cold = t.elapsed();
    index.save_cache(&cache).expect("save");
    assert_eq!(stats.parsed, NOTES);

    let t = Instant::now();
    let (mut index, stats) = Index::load(&root, &[], Some(&cache)).expect("warm load");
    let warm = t.elapsed();
    assert_eq!(stats.reused, NOTES);

    let rel = "dir00/sub0/note-0000.md";
    let path = root.join(rel);
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, format!("{text}[[note-0001]]\n")).expect("edit");
    let t = Instant::now();
    let change = index
        .refresh(&BTreeSet::from([rel.to_string()]))
        .expect("refresh")
        .expect("change");
    let refresh = t.elapsed();
    assert_eq!(change.modified, [rel]);

    println!("cold {cold:?}  warm {warm:?}  refresh {refresh:?}");
    println!(
        "warm: scan {:.0} ms, cache read {:.0} ms, resolve {:.0} ms",
        stats.scan_ms, stats.cache_read_ms, stats.resolve_ms
    );
    assert!(cold.as_secs_f64() < 2.0, "cold load {cold:?}");
    assert!(warm.as_secs_f64() < 1.0, "warm load {warm:?}");
    assert!(refresh.as_secs_f64() < 0.3, "refresh {refresh:?}");
    std::fs::remove_dir_all(root).ok();
}
