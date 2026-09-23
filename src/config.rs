//! Config file and root resolution.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub exclude: Vec<String>,
    pub graph_hops: u32,
    pub editor: String,
    pub send_max_bytes: usize,
    pub root: Vec<RootConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            graph_hops: 2,
            editor: String::new(),
            send_max_bytes: 65536,
            root: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootConfig {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub send_allow: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Root {
    pub name: Option<String>,
    pub path: PathBuf,
    pub send_allow: Vec<String>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(dir) = var("HERDR_PLUGIN_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("config.toml"));
    }
    let base = var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("knapp").join("config.toml"))
}

/// The plugin state dir under herdr, else `$XDG_CACHE_HOME/knapp`, else
/// `~/.cache/knapp`.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = var("HERDR_PLUGIN_STATE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("knapp"))
}

impl Config {
    pub fn load() -> Result<Self, String> {
        match config_path() {
            Some(path) if path.exists() => Self::from_file(&path),
            _ => Ok(Self::default()),
        }
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Configured roots with `~` expanded and relative paths joined to `base`
    /// (the workspace directory, or the current directory outside herdr).
    pub fn roots(&self, base: &Path) -> Vec<Root> {
        self.root
            .iter()
            .map(|r| Root {
                name: Some(r.name.clone()),
                path: expand(&r.path, base),
                send_allow: r.send_allow.clone(),
            })
            .collect()
    }

    /// The root for a CLI command: `--root NAME|PATH` when given, else the
    /// configured root containing `inside`, else `base` as an unconfigured
    /// root that cannot send.
    pub fn pick_root(&self, arg: Option<&str>, inside: &Path, base: &Path) -> Result<Root, String> {
        let roots = self.roots(base);
        if let Some(arg) = arg {
            if let Some(root) = roots.iter().find(|r| r.name.as_deref() == Some(arg)) {
                return Ok(root.clone());
            }
            return Ok(unconfigured(expand(arg, base)));
        }
        let inside = canonical(inside);
        for root in &roots {
            if inside.starts_with(canonical(&root.path)) {
                return Ok(root.clone());
            }
        }
        Ok(unconfigured(base.to_path_buf()))
    }
}

fn unconfigured(path: PathBuf) -> Root {
    Root {
        name: None,
        path,
        send_allow: Vec::new(),
    }
}

pub fn expand(path: &str, base: &Path) -> PathBuf {
    let path = match (path.strip_prefix("~/"), var("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ if path == "~" => var("HOME").map(PathBuf::from).unwrap_or_default(),
        _ => PathBuf::from(path),
    };
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
