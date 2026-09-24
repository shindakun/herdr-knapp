//! `open-pane`: open, focus, or close the notes pane in the focused tab.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The `[[panes]] title` of the notes pane in herdr-plugin.toml.
pub const PANE_TITLE: &str = "Knapp";

#[derive(Debug, Deserialize)]
struct Listing {
    result: PaneList,
}

#[derive(Debug, Deserialize)]
struct PaneList {
    panes: Vec<Pane>,
}

#[derive(Debug, Deserialize)]
struct Pane {
    pane_id: String,
    tab_id: String,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Open beside this pane.
    Open {
        beside: String,
    },
    Focus(String),
    Close(String),
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// From `herdr pane list` JSON: a knapp pane in the focused pane's tab is a
/// pane labelled `Knapp` whose cwd is the plugin root.
pub fn decide(json: &str, plugin_root: &Path) -> Result<Decision, String> {
    let listing: Listing = serde_json::from_str(json).map_err(|e| format!("pane list: {e}"))?;
    let panes = listing.result.panes;
    let focused = panes.iter().find(|p| p.focused).ok_or("no focused pane")?;
    let root = canonical(plugin_root);
    let ours = panes.iter().find(|p| {
        p.tab_id == focused.tab_id
            && p.label.as_deref() == Some(PANE_TITLE)
            && p.cwd
                .as_deref()
                .is_some_and(|c| canonical(Path::new(c)) == root)
    });
    let checked = |id: &str| {
        if crate::send::valid_pane_id(id) {
            Ok(id.to_string())
        } else {
            Err(format!("unexpected pane id `{id}`"))
        }
    };
    match ours {
        None => Ok(Decision::Open {
            beside: checked(&focused.pane_id)?,
        }),
        Some(p) if p.focused => Ok(Decision::Close(checked(&p.pane_id)?)),
        Some(p) => Ok(Decision::Focus(checked(&p.pane_id)?)),
    }
}
