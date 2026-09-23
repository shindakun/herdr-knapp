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

const REPO: &str = env!("CARGO_MANIFEST_DIR");

/// Runs knapp from the repo root with empty config and cache dirs, so a
/// user's config cannot change the result and their cache is never touched.
fn knapp_isolated(args: &[&str]) -> std::process::Output {
    let base = std::env::temp_dir().join(format!("knapp-test-{}", std::process::id()));
    let (config, cache) = (base.join("config"), base.join("cache"));
    std::fs::create_dir_all(&config).expect("create config dir");
    std::fs::create_dir_all(&cache).expect("create cache dir");
    Command::new(env!("CARGO_BIN_EXE_knapp"))
        .args(args)
        .current_dir(REPO)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_CACHE_HOME", &cache)
        .env_remove("HERDR_PLUGIN_CONFIG_DIR")
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .output()
        .expect("run knapp")
}

fn files_under(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("read fixture dir")
        .map(|e| e.expect("dir entry"))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let rel = format!("{prefix}{name}");
        if entry.file_type().expect("file type").is_dir() {
            files_under(&entry.path(), &format!("{rel}/"), out);
        } else {
            out.push(rel);
        }
    }
}

#[test]
fn fixtures_match_expected_output() {
    let fixtures = std::path::Path::new(REPO).join("fixtures");
    let expected_dir = std::path::Path::new(REPO).join("tests/expected");
    let mut unused: std::collections::BTreeSet<String> = std::fs::read_dir(&expected_dir)
        .expect("read tests/expected")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    let mut failures = Vec::new();
    let mut check = |args: Vec<String>, expected_name: String| {
        let path = expected_dir.join(&expected_name);
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        unused.remove(&expected_name);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = knapp_isolated(&refs);
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || stdout != expected {
            failures.push(format!(
                "knapp {}\n--- expected ({expected_name})\n{expected}--- got (exit {})\n{stdout}{}",
                args.join(" "),
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    };

    let mut fixture_names: Vec<_> = std::fs::read_dir(&fixtures)
        .expect("read fixtures")
        .map(|e| e.expect("entry"))
        .filter(|e| e.file_type().expect("file type").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    fixture_names.sort();
    for fixture in fixture_names {
        let root = format!("fixtures/{fixture}");
        let mut files = Vec::new();
        files_under(&fixtures.join(&fixture), "", &mut files);
        for rel in files {
            let part = rel.replace('/', "_");
            let path = format!("{root}/{rel}");
            if rel.ends_with(".md") {
                check(
                    vec!["links".into(), "--root".into(), root.clone(), path.clone()],
                    format!("{fixture}.links.{part}.txt"),
                );
            }
            check(
                vec!["backlinks".into(), "--root".into(), root.clone(), path],
                format!("{fixture}.backlinks.{part}.txt"),
            );
        }
        check(
            vec!["unresolved".into(), "--root".into(), root.clone()],
            format!("{fixture}.unresolved.txt"),
        );
    }
    assert!(
        unused.is_empty(),
        "expected files with no fixture file: {unused:?}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
