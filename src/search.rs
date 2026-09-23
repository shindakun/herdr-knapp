//! Full-text search: ripgrep when it is on `PATH`, else built in.

use std::io::{BufRead, BufReader};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_RESULTS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub rel: String,
    pub line: u32,
    /// The matching line, without its line ending.
    pub text: String,
    /// Byte ranges of the matches within `text`.
    pub ranges: Vec<Range<usize>>,
}

pub fn find_rg() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("rg"))
        .find(|p| p.is_file())
}

fn case_sensitive(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

/// Runs a search, calling `sink` with each match until it returns false,
/// `cancel` is set, or `MAX_RESULTS` is reached. `notes` are the indexed
/// notes the built-in search reads.
pub fn search(
    root: &Path,
    query: &str,
    exclude: &[String],
    notes: &[String],
    rg: Option<&Path>,
    cancel: &AtomicBool,
    mut sink: impl FnMut(Match) -> bool,
) -> Result<(), String> {
    if query.is_empty() {
        return Ok(());
    }
    let mut count = 0;
    let mut take = |m: Match| {
        count += 1;
        sink(m) && count < MAX_RESULTS && !cancel.load(Ordering::Relaxed)
    };
    match rg {
        Some(rg) => ripgrep(rg, root, query, exclude, cancel, &mut take),
        None => builtin(root, query, notes, cancel, &mut take),
    }
}

fn ripgrep(
    rg: &Path,
    root: &Path,
    query: &str,
    exclude: &[String],
    cancel: &AtomicBool,
    take: &mut dyn FnMut(Match) -> bool,
) -> Result<(), String> {
    let mut cmd = Command::new(rg);
    cmd.args(["--json", "--fixed-strings", "--smart-case", "--no-ignore"])
        .args(["--iglob", "*.md"]);
    for e in exclude.iter().filter(|e| e.ends_with('/')) {
        cmd.arg("--glob").arg(format!("!{e}**"));
    }
    // An explicit path and a null stdin: with neither, rg searches stdin
    // whenever stdin is not a terminal, and waits forever.
    cmd.arg("--")
        .arg(query)
        .arg(".")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("rg: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        if let Some(m) = parse_rg_match(&line) {
            if !take(m) {
                break;
            }
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

/// One `rg --json` line to a match; `None` for other records and for
/// non-UTF-8 paths (given as `path.bytes`).
pub fn parse_rg_match(line: &str) -> Option<Match> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "match" {
        return None;
    }
    let data = v.get("data")?;
    let rel = data.get("path")?.get("text")?.as_str()?;
    let rel = rel.strip_prefix("./").unwrap_or(rel).to_string();
    let text = data.get("lines")?.get("text")?.as_str()?;
    let text = text.trim_end_matches(['\n', '\r']).to_string();
    let ranges = data
        .get("submatches")?
        .as_array()?
        .iter()
        .filter_map(|s| {
            let start = s.get("start")?.as_u64()? as usize;
            let end = s.get("end")?.as_u64()? as usize;
            (end <= text.len()).then_some(start..end)
        })
        .collect();
    Some(Match {
        rel,
        line: data.get("line_number")?.as_u64()? as u32,
        text,
        ranges,
    })
}

fn builtin(
    root: &Path,
    query: &str,
    notes: &[String],
    cancel: &AtomicBool,
    take: &mut dyn FnMut(Match) -> bool,
) -> Result<(), String> {
    let sensitive = case_sensitive(query);
    let needle = if sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    for rel in notes {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let hay = if sensitive {
                line.to_string()
            } else {
                line.to_lowercase()
            };
            if !hay.contains(&needle) {
                continue;
            }
            // Offsets in the lowercased line are only valid when lowercasing
            // kept every byte length; otherwise the line is shown unmarked.
            let ranges = if hay.len() == line.len() {
                hay.match_indices(&needle)
                    .map(|(s, m)| s..s + m.len())
                    .collect()
            } else {
                Vec::new()
            };
            let m = Match {
                rel: rel.clone(),
                line: i as u32 + 1,
                text: line.to_string(),
                ranges,
            };
            if !take(m) {
                return Ok(());
            }
        }
    }
    Ok(())
}
