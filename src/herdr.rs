//! Herdr context JSON, pane open, agent prompt, and graphics calls.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The fields of `HERDR_PLUGIN_CONTEXT_JSON` knapp reads.
#[derive(Debug, Default, Deserialize)]
pub struct Context {
    pub workspace_id: Option<String>,
    pub workspace_cwd: Option<String>,
    pub focused_pane_id: Option<String>,
}

impl Context {
    pub fn from_env() -> Option<Self> {
        serde_json::from_str(&var("HERDR_PLUGIN_CONTEXT_JSON")?).ok()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Deserialize)]
struct AgentListing {
    result: AgentList,
}

#[derive(Deserialize)]
struct AgentList {
    agents: Vec<Agent>,
}

pub fn parse_agent_list(json: &str) -> Result<Vec<Agent>, String> {
    serde_json::from_str::<AgentListing>(json)
        .map(|l| l.result.agents)
        .map_err(|e| format!("agent list: {e}"))
}

fn herdr(args: &[&str]) -> Result<String, String> {
    let bin = var("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let out = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn agents() -> Result<Vec<Agent>, String> {
    parse_agent_list(&herdr(&["agent", "list"])?)
}

/// The workspace's agent: the focused pane if it is one, else the focused
/// agent, else the first.
pub fn pick_agent<'a>(
    agents: &'a [Agent],
    workspace_id: &str,
    focused_pane: Option<&str>,
) -> Option<&'a Agent> {
    let mine: Vec<&Agent> = agents
        .iter()
        .filter(|a| a.workspace_id == workspace_id)
        .collect();
    mine.iter()
        .find(|a| Some(a.pane_id.as_str()) == focused_pane)
        .or_else(|| mine.iter().find(|a| a.focused))
        .or_else(|| mine.first())
        .copied()
}

/// Herdr's managed plugin directory, from `HERDR_PLUGIN_CONFIG_DIR`
/// (`.../herdr/plugins/config/<id>`).
fn plugins_dir() -> Option<PathBuf> {
    let config = PathBuf::from(var("HERDR_PLUGIN_CONFIG_DIR")?);
    config
        .ancestors()
        .find(|a| a.file_name().is_some_and(|n| n == "plugins"))
        .map(Path::to_path_buf)
}

/// The directory a pane's relative roots resolve against, when herdr
/// launched it. `workspace_cwd` is the focused pane's directory, which is
/// another plugin's checkout when that pane is a plugin pane, so the
/// workspace's agent comes first and herdr's plugin directory is skipped.
pub fn workspace_dir(ctx: &Context, agents: &[Agent]) -> Option<PathBuf> {
    let plugins = plugins_dir();
    let usable = |p: Option<&str>| {
        p.map(PathBuf::from)
            .filter(|p| p.is_dir() && !plugins.as_ref().is_some_and(|d| p.starts_with(d)))
    };
    let agent = ctx
        .workspace_id
        .as_deref()
        .and_then(|w| pick_agent(agents, w, ctx.focused_pane_id.as_deref()));
    usable(agent.and_then(|a| a.cwd.as_deref())).or_else(|| usable(ctx.workspace_cwd.as_deref()))
}
