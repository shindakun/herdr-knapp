//! Subcommands.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{self, Config};
use crate::index::{self, Fragment, Index, LoadStats, Resolved};
use crate::scan::Kind;

pub const USAGE: &str = "usage: knapp <command> [args]

commands:
  links FILE [--root NAME|PATH]           forward links, with resolution state
  backlinks FILE [--root NAME|PATH]       notes linking to FILE
  unresolved [--root NAME|PATH] [--json]  unresolved and ambiguous targets
  orphans [--root NAME|PATH]              notes nothing links to
  tags [--root NAME|PATH]                 tags with note counts
  graph FILE [--root NAME|PATH] [--hops N] [--dot]
                                          the local graph as a tree, or Graphviz
  pane [--root NAME|PATH]                 browse the notes in a terminal pane
  peek                                    the note in KNAPP_NOTE (herdr popup)
  open-pane                               herdr action: open, focus, or close the pane
  peek-selection                          herdr action: peek at a clicked or selected note
  index [--root NAME|PATH] [--rebuild] [--stats] [--watch]
                                          load the root and write the cache
  help                                    show this message
  version                                 print the version";

#[derive(Default)]
struct Args {
    file: Option<String>,
    root: Option<String>,
    hops: Option<u32>,
    flags: BTreeSet<&'static str>,
}

/// `flags` lists the boolean flags this command accepts, such as `--json`.
fn parse_args(args: &[String], wants_file: bool, flags: &[&'static str]) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--hops" && flags.contains(&"--hops") {
            let v = it.next().ok_or("--hops needs a value")?;
            out.hops = Some(
                v.parse()
                    .map_err(|_| format!("--hops: not a number: {v}"))?,
            );
            continue;
        }
        if let Some(flag) = flags.iter().find(|f| **f == arg) {
            out.flags.insert(flag);
            continue;
        }
        match arg.as_str() {
            "--root" => out.root = Some(it.next().ok_or("--root needs a value")?.clone()),
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

struct Opened {
    index: Index,
    rel: Option<String>,
    stats: LoadStats,
    cache: Option<PathBuf>,
    cache_write_ms: f64,
}

/// Loads the index for the root that applies to `file` (or the current
/// directory), writing the cache when it went stale.
fn open(args: &Args) -> Result<Opened, String> {
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
    let canonical = config::canonical(&root.path);
    let cache = config::cache_dir().map(|d| index::cache_path(&d, &canonical));
    if args.flags.contains("--rebuild") {
        if let Some(c) = &cache {
            let _ = std::fs::remove_file(c);
        }
    }
    let (index, stats) = Index::load(&root.path, &config.exclude, cache.as_deref())?;
    let mut cache_write_ms = 0.0;
    if let (Some(c), true) = (&cache, stats.stale) {
        let t = Instant::now();
        if let Err(e) = index.save_cache(c) {
            eprintln!("knapp: cache not written: {e}");
        }
        cache_write_ms = t.elapsed().as_secs_f64() * 1e3;
    }
    let rel = match file {
        Some(f) => Some(relative(&index.root, &f)?),
        None => None,
    };
    Ok(Opened {
        index,
        rel,
        stats,
        cache,
        cache_write_ms,
    })
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
    let args = parse_args(args, true, &[])?;
    let Opened { index, rel, .. } = open(&args)?;
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
    let args = parse_args(args, true, &[])?;
    let Opened { index, rel, .. } = open(&args)?;
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

pub fn unresolved(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false, &["--json"])?;
    let Opened { index, .. } = open(&args)?;
    let groups = index.unresolved();
    let state = |g: &crate::index::Target| {
        if g.ambiguous {
            "ambiguous"
        } else {
            "unresolved"
        }
    };
    let rel = |id: usize| index.files[id].rel.clone();
    if args.flags.contains("--json") {
        let json: Vec<serde_json::Value> = groups
            .iter()
            .map(|g| {
                serde_json::json!({
                    "target": g.shown,
                    "state": state(g),
                    "count": g.sources.len(),
                    "sources": g.sources.iter().map(|s| serde_json::json!({
                        "path": rel(s.file),
                        "line": s.line,
                        "goes_to": s.pick.map(rel),
                    })).collect::<Vec<_>>(),
                    "candidates": g.candidates.iter().map(|&c| rel(c)).collect::<Vec<_>>(),
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
            out.push_str(&format!("{}\t{}\t{}\n", g.sources.len(), state(g), g.shown));
        }
        print!("{out}");
    }
    Ok(())
}

pub fn orphans(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false, &[])?;
    let Opened { index, .. } = open(&args)?;
    let mut out = String::new();
    for id in index.orphans() {
        out.push_str(&index.files[id].rel);
        out.push('\n');
    }
    print!("{out}");
    Ok(())
}

pub fn tags(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false, &[])?;
    let Opened { index, .. } = open(&args)?;
    fn walk(nodes: &[crate::index::TagNode], out: &mut String) {
        for n in nodes {
            out.push_str(&format!("{}\t{}\n", n.count, n.shown));
            walk(&n.children, out);
        }
    }
    let mut out = String::new();
    walk(&index.tags(), &mut out);
    print!("{out}");
    Ok(())
}

pub fn index(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false, &["--rebuild", "--stats", "--watch"])?;
    let Opened {
        mut index,
        stats,
        cache,
        cache_write_ms,
        ..
    } = open(&args)?;
    if args.flags.contains("--stats") {
        print!("{}", stats_text(&index, &stats, cache_write_ms));
    } else {
        let notes = index.files.iter().filter(|f| f.kind == Kind::Note).count();
        let links: usize = index.forward.iter().map(Vec::len).sum();
        println!(
            "{notes} notes, {} attachments, {links} links",
            index.files.len() - notes
        );
    }
    if !args.flags.contains("--watch") {
        return Ok(());
    }
    let ignore = cache.as_deref().and_then(Path::parent);
    let watch = crate::watch::watch(&index.root, ignore)?;
    let mut stdout = std::io::stdout();
    while let Some(touched) = watch.next_batch() {
        let t = Instant::now();
        if let Some(change) = index.refresh(&touched)? {
            let _ = writeln!(
                stdout,
                "+{} -{} ~{} {:.0} ms",
                change.added.len(),
                change.removed.len(),
                change.modified.len(),
                t.elapsed().as_secs_f64() * 1e3
            );
            let _ = stdout.flush();
        }
    }
    Ok(())
}

fn stats_text(index: &Index, stats: &LoadStats, cache_write_ms: f64) -> String {
    let notes = index.files.iter().filter(|f| f.kind == Kind::Note).count();
    let excluded = index.files.iter().filter(|f| f.excluded).count();
    let (mut resolved, mut ambiguous, mut unresolved) = (0, 0, 0);
    for r in index.forward.iter().flatten() {
        match r {
            Resolved::File { .. } => resolved += 1,
            Resolved::Ambiguous { .. } => ambiguous += 1,
            Resolved::Unresolved => unresolved += 1,
        }
    }
    let tags: BTreeSet<&str> = index
        .files
        .iter()
        .filter_map(|f| f.parsed.as_ref())
        .flat_map(|p| p.tags.iter().map(|t| t.name.as_str()))
        .collect();
    let mut out = format!(
        "notes\t{notes}\nattachments\t{}\nexcluded\t{excluded}\n\
         links.resolved\t{resolved}\nlinks.ambiguous\t{ambiguous}\nlinks.unresolved\t{unresolved}\n\
         tags\t{}\ncache.reused\t{}\ncache.parsed\t{}\n\
         ms.scan\t{:.1}\nms.cache_read\t{:.1}\nms.parse\t{:.1}\nms.resolve\t{:.1}\nms.cache_write\t{:.1}\n",
        index.files.len() - notes,
        tags.len(),
        stats.reused,
        stats.parsed,
        stats.scan_ms,
        stats.cache_read_ms,
        stats.parse_ms,
        stats.resolve_ms,
        cache_write_ms,
    );
    for f in &index.skipped_filters {
        out.push_str(&format!("skipped_filter\t{f}\n"));
    }
    out
}

pub fn pane(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, false, &[])?;
    crate::tui::run(args.root.as_deref())
}

pub fn graph(args: &[String]) -> Result<(), String> {
    let args = parse_args(args, true, &["--hops", "--dot"])?;
    let hops = match args.hops {
        Some(h) => h,
        None => Config::load()?.graph_hops,
    };
    let Opened { index, rel, .. } = open(&args)?;
    let id = file_id(&index, &rel.expect("graph takes a file"))?;
    let local = crate::graph::local(&index, id, hops, None);
    if args.flags.contains("--dot") {
        print!("{}", crate::graph::dot(&index, &local));
    } else {
        let mut out = String::new();
        for line in crate::graph::tree(&index, &local) {
            out.push_str(&line);
            out.push('\n');
        }
        print!("{out}");
    }
    Ok(())
}

/// `open-pane` (the `open` action): open, focus, or close the notes pane in
/// the focused tab.
pub fn open_pane(_args: &[String]) -> Result<(), String> {
    use crate::launch::{decide, Decision};
    let root = std::env::var("HERDR_PLUGIN_ROOT")
        .map(PathBuf::from)
        .or_else(|_| std::env::current_dir())
        .map_err(|e| format!("plugin root: {e}"))?;
    match decide(&crate::herdr::pane_list()?, &root)? {
        Decision::Focus(id) => crate::herdr::pane_focus(&id),
        Decision::Close(id) => crate::herdr::pane_close(&id),
        Decision::Open { beside } => {
            let ctx = crate::herdr::Context::from_env().unwrap_or_default();
            let agents = crate::herdr::agents().unwrap_or_default();
            let dir = crate::herdr::workspace_dir(&ctx, &agents);
            let dir = dir.map(|d| d.display().to_string());
            let env: Vec<(&str, &str)> = dir
                .as_deref()
                .map(|d| ("KNAPP_CWD", d))
                .into_iter()
                .collect();
            crate::herdr::open_pane("notes", Some(&beside), &env)
        }
    }
}

/// `peek-selection` (the link handler's action, and a keybinding over a
/// selection): open the peek popup on the clicked or selected note.
/// Actions have no screen, so a failure opens the popup with the reason.
/// herdr suppresses notifications for the active tab, which is where every
/// click comes from, so a notification is only the fallback when the popup
/// itself cannot open (`ui_busy`).
pub fn peek_selection(_args: &[String]) -> Result<(), String> {
    let result = match peek_target() {
        Ok(t) => {
            let path = t.path.display().to_string();
            let mut env = vec![("KNAPP_NOTE", path.as_str())];
            if let Some(f) = &t.fragment {
                env.push(("KNAPP_FRAGMENT", f.as_str()));
            }
            crate::herdr::open_pane("peek", None, &env)
        }
        Err(reason) => {
            let shown = crate::herdr::open_pane("peek", None, &[("KNAPP_ERROR", reason.as_str())]);
            Err(shown.err().unwrap_or(reason))
        }
    };
    if let Err(e) = &result {
        if e.starts_with("ui_busy") {
            let _ = crate::herdr::notify("knapp", e);
        }
    }
    result
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn peek_target() -> Result<crate::peek::Target, String> {
    use crate::peek::{clean_selection, from_url, Target};
    let ctx = crate::herdr::Context::from_env().ok_or("peek needs herdr's context")?;
    if let Some(url) = ctx.clicked_url.as_deref() {
        let t = from_url(url, &hostname())?;
        return if t.path.is_file() {
            Ok(t)
        } else {
            Err(format!("no note at {}", t.path.display()))
        };
    }
    let text = ctx.selected_text.as_deref().unwrap_or("");
    let (target, fragment) = clean_selection(text);
    if target.is_empty() {
        return Err("nothing selected".into());
    }
    let agents = crate::herdr::agents().unwrap_or_default();
    let base = crate::herdr::workspace_dir(&ctx, &agents);
    let found = |path: PathBuf| Target {
        path,
        fragment: fragment.clone(),
    };
    let as_path = Path::new(&target);
    if as_path.is_absolute() && as_path.is_file() {
        return Ok(found(as_path.to_path_buf()));
    }
    if let Some(b) = &base {
        for candidate in [b.join(&target), b.join(format!("{target}.md"))] {
            if candidate.is_file() {
                return Ok(found(candidate));
            }
        }
    }
    // A wikilink target, resolved from the top of each configured root.
    let config = Config::load()?;
    let cache_dir = config::cache_dir();
    for root in config.roots(base.as_deref().unwrap_or(Path::new("/"))) {
        let canonical = config::canonical(&root.path);
        let cache = cache_dir
            .as_deref()
            .map(|d| index::cache_path(d, &canonical));
        let Ok((idx, _)) = Index::load(&root.path, &config.exclude, cache.as_deref()) else {
            continue;
        };
        if let [id] = idx.wikilink_from(&target, "")[..] {
            return Ok(found(idx.root.join(&idx.files[id].rel)));
        }
    }
    Err(format!("no note named {target}"))
}

/// `peek` (the popup): the note in `KNAPP_NOTE`, at `KNAPP_FRAGMENT`.
pub fn peek(_args: &[String]) -> Result<(), String> {
    if let Some(reason) = std::env::var("KNAPP_ERROR").ok().filter(|r| !r.is_empty()) {
        return crate::tui::show_message(&reason);
    }
    let note = std::env::var("KNAPP_NOTE").map_err(|_| "KNAPP_NOTE is not set")?;
    let fragment = std::env::var("KNAPP_FRAGMENT")
        .ok()
        .filter(|f| !f.is_empty());
    crate::tui::run_peek(Path::new(&note), fragment.as_deref())
}
