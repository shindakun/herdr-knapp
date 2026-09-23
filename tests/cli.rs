use std::process::Command;

fn knapp(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_knapp"))
        .args(args)
        .output()
        .expect("run knapp")
}

#[test]
fn help_exits_zero() {
    let out = knapp(&["help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("usage: knapp"));
}

#[test]
fn version_matches_manifest() {
    let out = knapp(&["version"]);
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml"))
            .expect("read herdr-plugin.toml");
    let want = format!("version = \"{}\"", env!("CARGO_PKG_VERSION"));
    assert!(
        manifest.lines().any(|l| l == want),
        "herdr-plugin.toml version differs from Cargo.toml"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("knapp {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_command_fails() {
    let out = knapp(&["frobnicate"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown command: frobnicate"));
}
