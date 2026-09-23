mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};

use knapp::herdr::{Graphics, Placement};

/// A fake herdr socket: records each request line and answers `info` with a
/// cell size and everything else with ok, or `feature_disabled` when told.
fn server(disabled: bool) -> (std::path::PathBuf, Arc<Mutex<Vec<serde_json::Value>>>) {
    let dir = common::temp_dir(if disabled { "gfx-off" } else { "gfx" });
    let path = dir.join("herdr.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let method = req["method"].as_str().unwrap_or("").to_string();
            seen.lock().unwrap().push(req);
            let reply = if disabled {
                r#"{"id":"knapp","error":{"code":"feature_disabled","message":"graphics are off"}}"#
                    .to_string()
            } else if method == "pane.graphics.info" {
                r#"{"id":"knapp","result":{"type":"pane_graphics_info","cell_width_px":8,"cell_height_px":16,"pane_visible":true}}"#.to_string()
            } else {
                r#"{"id":"knapp","result":{"type":"ok"}}"#.to_string()
            };
            writeln!(stream, "{reply}").unwrap();
        }
    });
    (path, log)
}

#[test]
fn info_set_and_clear_requests() {
    let (sock, log) = server(false);
    let g = Graphics::new(sock, "w1:p3".into());
    let info = g.info().unwrap();
    assert_eq!(info.cell_px, (8, 16));
    assert!(info.visible);
    let png = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/vault-basic/img.png"
    ))
    .unwrap();
    g.set(
        "graph",
        &png,
        (2, 2),
        Placement {
            col: 30,
            row: -2,
            cols: 40,
            rows: 12,
        },
    )
    .unwrap();
    g.clear("graph").unwrap();

    let log = log.lock().unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(log[0]["method"], "pane.graphics.info");
    assert_eq!(log[0]["params"]["pane_id"], "w1:p3");
    let set = &log[1]["params"];
    assert_eq!(log[1]["method"], "pane.graphics.set");
    assert_eq!(set["layer_id"], "graph");
    assert_eq!(set["z_index"], -1);
    assert_eq!(set["format"], "png");
    assert_eq!(
        (set["image_width"].as_u64(), set["image_height"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(set["data_base64"], knapp::editor::base64(&png));
    assert_eq!(set["placement"]["viewport_row"], -2);
    assert_eq!(set["placement"]["grid_cols"], 40);
    assert_eq!(log[2]["method"], "pane.graphics.clear");
    assert_eq!(log[2]["params"]["layer_id"], "graph");
}

#[test]
fn errors_come_back_as_code_and_message() {
    let (sock, _) = server(true);
    let g = Graphics::new(sock, "w1:p3".into());
    assert_eq!(g.info().unwrap_err(), "feature_disabled: graphics are off");
}
