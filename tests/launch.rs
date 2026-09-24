use std::path::Path;

use knapp::launch::{decide, Decision};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// The fixture with `focused` moved to `id`, and the plugin root filled in.
fn list(focus: &str) -> String {
    let raw = std::fs::read_to_string(format!("{ROOT}/tests/fixtures/herdr/pane_list.json"))
        .unwrap()
        .replace("PLUGIN_ROOT", ROOT);
    let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    for p in v["result"]["panes"].as_array_mut().unwrap() {
        p["focused"] = serde_json::Value::Bool(p["pane_id"] == focus);
    }
    v.to_string()
}

#[test]
fn focus_close_and_open() {
    let root = Path::new(ROOT);
    // Tab w1:t1 has a knapp pane (w1:p2) that is not focused.
    assert_eq!(
        decide(&list("w1:p1"), root).unwrap(),
        Decision::Focus("w1:p2".into())
    );
    assert_eq!(
        decide(&list("w1:p2"), root).unwrap(),
        Decision::Close("w1:p2".into())
    );
    // In w1:t2, w1:p4 is labelled Knapp but runs elsewhere: another
    // plugin's pane. w1:p3 is ours.
    assert_eq!(
        decide(&list("w1:p4"), root).unwrap(),
        Decision::Focus("w1:p3".into())
    );
}

#[test]
fn opens_beside_the_focused_pane_when_none() {
    let raw = list("w1:p1").replace("\"label\":\"Knapp\"", "\"label\":\"Other\"");
    assert_eq!(
        decide(&raw, Path::new(ROOT)).unwrap(),
        Decision::Open {
            beside: "w1:p1".into()
        }
    );
}

#[test]
fn odd_pane_ids_never_reach_an_argv() {
    let raw = list("w1:p1").replace("\"w1:p2\"", "\"w1:p2 --x\"");
    assert!(decide(&raw, Path::new(ROOT)).is_err());
}
