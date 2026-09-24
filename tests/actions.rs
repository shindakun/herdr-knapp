mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::temp_dir;

const REPO: &str = env!("CARGO_MANIFEST_DIR");

/// A fake herdr: logs each call's argv (joined by \x1f, one call a line)
/// and answers `pane list`, `agent list`, and everything else with ok.
/// `FAKE_PANES` picks the pane list; `FAKE_BUSY` makes `plugin pane open`
/// answer `ui_busy`.
struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn new(name: &str, agent_cwd: &Path) -> Self {
        let dir = temp_dir(name);
        let panes = std::fs::read_to_string(format!("{REPO}/tests/fixtures/herdr/pane_list.json"))
            .unwrap()
            .replace("PLUGIN_ROOT", REPO);
        std::fs::write(dir.join("panes.json"), &panes).unwrap();
        std::fs::write(
            dir.join("panes-none.json"),
            panes.replace("\"label\":\"Knapp\"", "\"label\":\"Other\""),
        )
        .unwrap();
        let agents =
            std::fs::read_to_string(format!("{REPO}/tests/fixtures/herdr/agent_list.json"))
                .unwrap()
                .replace("AGENT_A", &agent_cwd.display().to_string())
                .replace("AGENT_B", &agent_cwd.display().to_string());
        std::fs::write(dir.join("agents.json"), agents).unwrap();
        let script = format!(
            r#"#!/bin/sh
d='{dir}'
( IFS="$(printf '\037')"; printf '%s\n' "$*" ) >> "$d/log"
case "$1 $2" in
  'pane list') cat "$d/${{FAKE_PANES:-panes.json}}" ;;
  'agent list') cat "$d/agents.json" ;;
  'plugin pane')
    if [ -n "$FAKE_BUSY" ]; then
      echo '{{"error":{{"code":"ui_busy","message":"a modal is open"}}}}' >&2; exit 1
    fi
    echo '{{"result":{{}}}}' ;;
  *) echo '{{"result":{{}}}}' ;;
esac
"#,
            dir = dir.display()
        );
        std::fs::write(dir.join("herdr"), script).unwrap();
        std::fs::set_permissions(
            dir.join("herdr"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        Fake { dir }
    }

    fn run(
        &self,
        cmd: &str,
        context: serde_json::Value,
        extra: &[(&str, &str)],
    ) -> std::process::Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_knapp"));
        c.arg(cmd)
            .current_dir(REPO)
            .env("HERDR_BIN_PATH", self.dir.join("herdr"))
            .env("HERDR_PLUGIN_ROOT", REPO)
            .env("HERDR_PLUGIN_ID", "shindakun.knapp")
            .env("HERDR_PLUGIN_CONTEXT_JSON", context.to_string())
            .env("HERDR_PLUGIN_CONFIG_DIR", self.dir.join("config"))
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env_remove("HERDR_PLUGIN_STATE_DIR");
        for (k, v) in extra {
            c.env(k, v);
        }
        c.output().unwrap()
    }

    /// Each logged call as its argv.
    fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.dir.join("log"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.split('\x1f').map(str::to_string).collect())
            .collect()
    }

    fn call(&self, first: &str, second: &str) -> Option<Vec<String>> {
        self.calls()
            .into_iter()
            .find(|c| c.len() > 1 && c[0] == first && c[1] == second)
    }
}

fn ctx(extra: serde_json::Value) -> serde_json::Value {
    let mut v = serde_json::json!({"workspace_id": "w1", "focused_pane_id": "w1:p1"});
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    v
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn open_pane_focuses_or_opens_beside_the_focused_pane() {
    let work = temp_dir("actions-work");
    let fake = Fake::new("actions-open", &work);
    let out = fake.run("open-pane", ctx(serde_json::json!({})), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fake.call("plugin", "pane"),
        Some(strs(&["plugin", "pane", "focus", "w1:p2"]))
    );

    let fake = Fake::new("actions-open-none", &work);
    let out = fake.run(
        "open-pane",
        ctx(serde_json::json!({})),
        &[("FAKE_PANES", "panes-none.json")],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let open = fake.call("plugin", "pane").expect("plugin pane open");
    let work = work.display().to_string();
    assert_eq!(
        open,
        strs(&[
            "plugin",
            "pane",
            "open",
            "--plugin",
            "shindakun.knapp",
            "--entrypoint",
            "notes",
            "--target-pane",
            "w1:p1",
            "--direction",
            "right",
            "--env",
            &format!("KNAPP_CWD={work}"),
        ])
    );
}

#[test]
fn peek_from_a_clicked_file_url() {
    let work = temp_dir("actions-peek-work");
    std::fs::write(work.join("my note.md"), "# Top\n").unwrap();
    let fake = Fake::new("actions-peek", &work);
    let url = format!("file://{}/my%20note.md#Top", work.display());
    let out = fake.run(
        "peek-selection",
        ctx(serde_json::json!({"clicked_url": url})),
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fake.call("plugin", "pane").unwrap(),
        strs(&[
            "plugin",
            "pane",
            "open",
            "--plugin",
            "shindakun.knapp",
            "--entrypoint",
            "peek",
            "--env",
            &format!("KNAPP_NOTE={}/my note.md", work.display()),
            "--env",
            "KNAPP_FRAGMENT=Top",
        ])
    );
}

#[test]
fn peek_from_a_selected_wikilink_through_the_config() {
    let work = temp_dir("actions-sel-work");
    let fake = Fake::new("actions-sel", &work);
    std::fs::create_dir_all(fake.dir.join("config")).unwrap();
    std::fs::write(
        fake.dir.join("config/config.toml"),
        format!("[[root]]\nname = \"basic\"\npath = \"{REPO}/fixtures/vault-basic\"\n"),
    )
    .unwrap();
    let out = fake.run(
        "peek-selection",
        ctx(serde_json::json!({"selected_text": "[[deep|x]]"})),
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let open = fake.call("plugin", "pane").unwrap();
    assert_eq!(
        open[8],
        format!("KNAPP_NOTE={REPO}/fixtures/vault-basic/dir/deep.md")
    );
}

#[test]
fn failures_open_the_popup_with_the_reason() {
    let work = temp_dir("actions-fail-work");
    std::fs::write(work.join("a.md"), "x\n").unwrap();

    let fake = Fake::new("actions-missing", &work);
    let out = fake.run(
        "peek-selection",
        ctx(serde_json::json!({"clicked_url": "file:///nowhere/x.md"})),
        &[],
    );
    assert!(!out.status.success());
    let open = fake.call("plugin", "pane").expect("the popup opens");
    assert_eq!(&open[5..8], ["--entrypoint", "peek", "--env"]);
    assert_eq!(open[8], "KNAPP_ERROR=no note at /nowhere/x.md");
    assert!(fake.call("notification", "show").is_none());

    let fake = Fake::new("actions-foreign", &work);
    let out = fake.run(
        "peek-selection",
        ctx(serde_json::json!({"clicked_url": "file://far-away/a.md"})),
        &[],
    );
    assert!(!out.status.success());
    assert!(fake.call("plugin", "pane").unwrap()[8].contains("another machine"));

    // The popup cannot open: a notification is all that is left.
    let fake = Fake::new("actions-busy", &work);
    let url = format!("file://{}/a.md", work.display());
    let out = fake.run(
        "peek-selection",
        ctx(serde_json::json!({"clicked_url": url})),
        &[("FAKE_BUSY", "1")],
    );
    assert!(!out.status.success());
    let note = fake.call("notification", "show").expect("a notification");
    assert_eq!(note[2], "knapp");
    assert!(note[4].starts_with("ui_busy"), "{note:?}");
}
