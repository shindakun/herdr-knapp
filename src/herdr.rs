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
    #[serde(default)]
    pub selected_text: Option<String>,
    #[serde(default)]
    pub clicked_url: Option<String>,
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
    /// The agent's kind, such as `claude` or `codex`.
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
}

impl Agent {
    pub fn label(&self) -> String {
        format!(
            "{} {}",
            self.agent.as_deref().unwrap_or("agent"),
            self.pane_id
        )
    }
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
        return Err(error_message(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Herdr prints errors as `{"error": {"code", "message"}}` on stderr.
pub fn error_message(stderr: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(stderr.trim()).ok();
    match parsed.as_ref().and_then(|v| v.get("error")) {
        Some(e) => {
            let code = e.get("code").and_then(|c| c.as_str()).unwrap_or("error");
            let message = e.get("message").and_then(|m| m.as_str()).unwrap_or("");
            format!("{code}: {message}")
        }
        None => stderr.trim().to_string(),
    }
}

/// `herdr agent prompt <pane> <text>`: pastes and submits. The text goes as
/// one plain argument; herdr's parser takes it as the prompt even when it
/// starts with `-`, and `--` would break it.
pub fn prompt(pane: &str, text: &str) -> Result<(), String> {
    herdr(&["agent", "prompt", pane, text]).map(drop)
}

pub fn pane_list() -> Result<String, String> {
    herdr(&["pane", "list"])
}

/// Focuses one of this plugin's panes. (`herdr pane focus` moves by
/// direction; it takes no pane id.)
pub fn pane_focus(id: &str) -> Result<(), String> {
    herdr(&["plugin", "pane", "focus", id]).map(drop)
}

pub fn pane_close(id: &str) -> Result<(), String> {
    herdr(&["plugin", "pane", "close", id]).map(drop)
}

/// `herdr plugin pane open` for one of knapp's panes, with `--env` pairs.
pub fn open_pane(
    entrypoint: &str,
    beside: Option<&str>,
    env: &[(&str, &str)],
) -> Result<(), String> {
    let plugin = var("HERDR_PLUGIN_ID").unwrap_or_else(|| "shindakun.knapp".into());
    let mut args: Vec<String> = vec![
        "plugin".into(),
        "pane".into(),
        "open".into(),
        "--plugin".into(),
        plugin,
        "--entrypoint".into(),
        entrypoint.into(),
    ];
    if let Some(p) = beside {
        args.extend([
            "--target-pane".into(),
            p.into(),
            "--direction".into(),
            "right".into(),
        ]);
    }
    for (k, v) in env {
        args.extend(["--env".into(), format!("{k}={v}")]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    herdr(&refs).map(drop)
}

pub fn notify(title: &str, body: &str) -> Result<(), String> {
    herdr(&["notification", "show", title, "--body", body]).map(drop)
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

/// Pane graphics over herdr's socket: one JSON request per line, one JSON
/// reply per line. The CLI has no graphics commands.
pub struct Graphics {
    socket: PathBuf,
    pane: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphicsInfo {
    pub cell_px: (u32, u32),
    pub visible: bool,
}

/// Where a layer goes, in cells relative to the pane. Rows and columns may
/// be negative; herdr clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Placement {
    pub col: i32,
    pub row: i32,
    pub cols: u16,
    pub rows: u16,
}

impl Graphics {
    /// Needs `HERDR_SOCKET_PATH` and `HERDR_PANE_ID`; a popup has no pane id.
    pub fn from_env() -> Option<Self> {
        Some(Self {
            socket: PathBuf::from(var("HERDR_SOCKET_PATH")?),
            pane: var("HERDR_PANE_ID")?,
        })
    }

    pub fn new(socket: PathBuf, pane: String) -> Self {
        Self { socket, pane }
    }

    fn call(
        &self,
        method: &str,
        mut params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use std::io::{BufRead, BufReader, Write};
        params["pane_id"] = serde_json::Value::String(self.pane.clone());
        let request = serde_json::json!({"id": "knapp", "method": method, "params": params});
        let mut stream = std::os::unix::net::UnixStream::connect(&self.socket)
            .map_err(|e| format!("{}: {e}", self.socket.display()))?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let mut line = request.to_string();
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .map_err(|e| format!("{method}: {e}"))?;
        let mut reply = String::new();
        BufReader::new(&stream)
            .read_line(&mut reply)
            .map_err(|e| format!("{method}: {e}"))?;
        let v: serde_json::Value =
            serde_json::from_str(&reply).map_err(|e| format!("{method}: {e}"))?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_str()).unwrap_or("error");
            let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
            return Err(format!("{code}: {message}"));
        }
        Ok(v.get("result").cloned().unwrap_or_default())
    }

    pub fn info(&self) -> Result<GraphicsInfo, String> {
        let r = self.call("pane.graphics.info", serde_json::json!({}))?;
        let num = |k: &str| r.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Ok(GraphicsInfo {
            cell_px: (num("cell_width_px"), num("cell_height_px")),
            visible: r
                .get("pane_visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    /// Places a PNG on `layer` under the pane's text (`z_index` -1).
    pub fn set(
        &self,
        layer: &str,
        png: &[u8],
        size: (u32, u32),
        at: Placement,
    ) -> Result<(), String> {
        self.call(
            "pane.graphics.set",
            serde_json::json!({
                "layer_id": layer,
                "z_index": -1,
                "format": "png",
                "image_width": size.0,
                "image_height": size.1,
                "data_base64": crate::editor::base64(png),
                "placement": {
                    "viewport_col": at.col,
                    "viewport_row": at.row,
                    "grid_cols": at.cols,
                    "grid_rows": at.rows,
                },
            }),
        )
        .map(drop)
    }

    pub fn clear(&self, layer: &str) -> Result<(), String> {
        self.call(
            "pane.graphics.clear",
            serde_json::json!({"layer_id": layer}),
        )
        .map(drop)
    }
}

/// Width and height from a PNG's IHDR chunk.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}
