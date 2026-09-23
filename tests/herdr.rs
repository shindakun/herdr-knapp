mod common;

use common::temp_dir;
use knapp::herdr::{parse_agent_list, pick_agent, workspace_dir, Context};

fn agents(a: &str, b: &str) -> Vec<knapp::herdr::Agent> {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/herdr/agent_list.json"
    ))
    .unwrap()
    .replace("AGENT_A", a)
    .replace("AGENT_B", b);
    parse_agent_list(&json).unwrap()
}

#[test]
fn picks_the_focused_pane_then_the_focused_agent() {
    let list = agents("/a", "/b");
    assert_eq!(
        pick_agent(&list, "w1", Some("w1:p1")).unwrap().pane_id,
        "w1:p1"
    );
    assert_eq!(
        pick_agent(&list, "w1", Some("w1:p9")).unwrap().pane_id,
        "w1:p4"
    );
    assert!(pick_agent(&list, "w9", None).is_none());
}

/// The case seen in a real session: the focused pane was another plugin's
/// pane, so `workspace_cwd` was that plugin's checkout.
#[test]
fn skips_plugin_checkouts_and_prefers_the_agent() {
    let base = temp_dir("herdr-ctx");
    let plugins = base.join("herdr/plugins");
    let checkout = plugins.join("github/other-plugin-abc123");
    let project = base.join("project");
    for d in [&checkout, &plugins.join("config/shindakun.knapp"), &project] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::env::set_var(
        "HERDR_PLUGIN_CONFIG_DIR",
        plugins.join("config/shindakun.knapp"),
    );

    let ctx = Context {
        workspace_id: Some("w1".into()),
        workspace_cwd: Some(checkout.display().to_string()),
        focused_pane_id: Some("w1:p3".into()),
    };
    let project_s = project.display().to_string();
    let list = agents(&project_s, &project_s);
    assert_eq!(workspace_dir(&ctx, &list), Some(project.clone()));

    // No agent: the plugin checkout is still refused.
    assert_eq!(workspace_dir(&ctx, &[]), None);

    // No agent, an ordinary workspace directory: it is used.
    let ctx = Context {
        workspace_cwd: Some(project_s),
        ..ctx
    };
    assert_eq!(workspace_dir(&ctx, &[]), Some(project));
}
