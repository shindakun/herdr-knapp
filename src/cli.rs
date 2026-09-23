//! Subcommands.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::index::{key, Fragment, Index, Resolved};
use crate::scan::Kind;

pub const USAGE: &str = "usage: knapp <command> [args]

commands:
  links FILE [--root NAME|PATH]           forward links, with resolution state
  backlinks FILE [--root NAME|PATH]       notes linking to FILE
  unresolved [--root NAME|PATH] [--json]  unresolved and ambiguous targets
  help                                    show this message
  version                                 print the version";

struct Args {
    file: Option<String>,
    root: Option<String>,
    json: bool,
}

fn parse_args(args: &[String], wants_file: bool, allows_json: bool) -> Result<Args, String> {
    let mut out = Args {
        file: None,
        root: None,
        json: false,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => out.root = Some(it.next().ok_or("--root needs a value")?.clone()),
            "--json" if allows_json => out.json = true,
            a if a.starts_with('-') => return Err(format!("unknown flag: {a}\n{USAGE}")),
            a if wants_file && out.file.is_none() => out.file = Some(a.to_string()),
            a => return Err(format!("unexpected argument: {a}\n{USAGE}")),
        }
    }
    if wants_file && out.file.is_none() {
        return Err(format!("missing FILE\n{USAGE}"));
    }
    Ok(out)
}

fn cwd() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("current directory: {e}"))
}

/// Loads the index for the root that applies to `file` (or the current
/// directory) and returns it with `file`'s root-relative path.
fn open(args: &Args) -> Result<(Index, Option<String>), String> {
    let cwd = cwd()?;
    let config = Config::load()?;
    let file = args.file.as_deref().map(|f| cwd.join(f));
    if let Some(f) = &file {
        if !f.exists() {
            return Err(format!("{}: no such file", f.display()));
        }
    }
    let inside = file
        .as_deref()
        .and_then(Path::parent)
        .map_or_else(|| cwd.clone(), Path::to_path_buf);
    let root = config.pick_root(args.root.as_deref(), &inside, &cwd)?;
    let index = Index::load(&root.path, &config.exclude)?;
    let rel = match file {
        Some(f) => Some(relative(&index.root, &f)?),
        None => None,
    };
    Ok((index, rel))
}

/// `file` relative to `root`, following symlinks in its folder but not in
/// the file itself.
fn relative(root: &Path, file: &Path) -> Result<String, String> {
    let name = file
        .file_name()
        .ok_or_else(|| format!("{}: not a file", file.display()))?;
    let parent = file.parent().unwrap_or(Path::new("."));
    let parent = std::fs::canonicalize(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let full = parent.join(name);
    let rel = full
        .strip_prefix(root)
        .map_err(|_| format!("{}: outside the root {}", file.display(), root.display()))?;
    Ok(rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn file_id(index: &Index, rel: &str) -> Result<usize, String> {
    index
        .id(rel)
        .ok_or_else(|| format!("{rel}: not in the index (hidden or excluded)"))
}

pub fn links(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, true, false)?;
    let (index, rel) = open(&args)?;
    let rel = rel.expect("links takes a file");
    let id = file_id(&index, &rel)?;
    if index.files[id].kind == Kind::Attachment {
        return Err(format!("{rel}: not a note"));
    }
    let mut out = String::new();
    for (link, resolved) in index.links(id).iter().zip(&index.forward[id]) {
        let (state, fragment, target) = match resolved {
            Resolved::File { id, fragment } => {
                ("resolved", *fragment, index.files[*id].rel.clone())
            }
            Resolved::Ambiguous {
                pick,
                others,
                fragment,
            } => (
                "ambiguous",
                *fragment,
                std::iter::once(pick)
                    .chain(others)
                    .map(|&c| index.files[c].rel.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            Resolved::Unresolved => ("unresolved", Fragment::None, "-".to_string()),
        };
        let fragment = match fragment {
            Fragment::None => "-",
            Fragment::Found => "ok",
            Fragment::Missing => "missing",
        };
        out.push_str(&format!(
            "{}\t{state}\t{fragment}\t{}\t{target}\n",
            link.line, link.written
        ));
    }
    print!("{out}");
    Ok(())
}

pub fn backlinks(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, true, false)?;
    let (index, rel) = open(&args)?;
    let id = file_id(&index, &rel.expect("backlinks takes a file"))?;
    let hits: BTreeSet<(&str, u32, usize)> = index.back[id]
        .iter()
        .map(|&(source, pos)| {
            (
                index.files[source].rel.as_str(),
                index.links(source)[pos].line,
                source,
            )
        })
        .collect();
    let mut out = String::new();
    let mut current: Option<(usize, Vec<String>)> = None;
    for (rel, line, source) in hits {
        if current.as_ref().map(|(s, _)| *s) != Some(source) {
            let text =
                std::fs::read_to_string(index.root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
            current = Some((source, text.split('\n').map(str::to_string).collect()));
        }
        let lines = &current.as_ref().expect("set above").1;
        let text = lines.get(line as usize - 1).map_or("", |l| l.trim());
        out.push_str(&format!("{rel}:{line}\t{text}\n"));
    }
    print!("{out}");
    Ok(())
}

struct Group {
    state: &'static str,
    key: String,
    target: String,
    candidates: Vec<String>,
    sources: Vec<(String, u32)>,
}

pub fn unresolved(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false, true)?;
    let (index, _) = open(&args)?;
    let mut groups: Vec<Group> = Vec::new();
    for (source, file) in index.files.iter().enumerate() {
        for (link, resolved) in index.links(source).iter().zip(&index.forward[source]) {
            let (state, candidates) = match resolved {
                Resolved::File { .. } => continue,
                Resolved::Unresolved => ("unresolved", Vec::new()),
                Resolved::Ambiguous { pick, others, .. } => (
                    "ambiguous",
                    std::iter::once(pick)
                        .chain(others)
                        .map(|&c| index.files[c].rel.clone())
                        .collect(),
                ),
            };
            let k = key(&link.target);
            let k = k.strip_suffix(".md").unwrap_or(&k).to_string();
            let source = (file.rel.clone(), link.line);
            match groups.iter_mut().find(|g| g.state == state && g.key == k) {
                Some(g) => g.sources.push(source),
                None => groups.push(Group {
                    state,
                    key: k,
                    target: link.target.clone(),
                    candidates,
                    sources: vec![source],
                }),
            }
        }
    }
    groups.sort_by(|a, b| {
        b.sources
            .len()
            .cmp(&a.sources.len())
            .then((a.state != "unresolved").cmp(&(b.state != "unresolved")))
            .then(a.key.cmp(&b.key))
    });
    if args.json {
        let json: Vec<serde_json::Value> = groups
            .iter()
            .map(|g| {
                serde_json::json!({
                    "target": g.target,
                    "state": g.state,
                    "count": g.sources.len(),
                    "sources": g.sources.iter().map(|(path, line)| serde_json::json!({"path": path, "line": line})).collect::<Vec<_>>(),
                    "candidates": g.candidates,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?
        );
    } else {
        let mut out = String::new();
        for g in &groups {
            out.push_str(&format!("{}\t{}\t{}\n", g.sources.len(), g.state, g.target));
        }
        print!("{out}");
    }
    Ok(())
}
