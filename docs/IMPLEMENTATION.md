# Implementation guide

How to build `docs/PLAN.md`, step by step. The plan says what knapp does;
this says how each part is built, what it is tested with, and when a step is
done. Step numbers match the plan's Order.

## Conventions

- Errors are `Result<T, String>`. `main` prints `knapp: <err>` to stderr and
  exits 1.
- No async runtime. The pane uses threads and one `mpsc` channel.
- Inside the index a path is root-relative, `/`-separated, and a `String`.
  `PathBuf` appears only where knapp touches the filesystem.
- Lookup keys are NFC-normalized and lowercased
  (`unicode-normalization`, then `str::to_lowercase`).
- Herdr is called through `HERDR_BIN_PATH` (default `herdr`), and through the
  socket only for graphics, which has no CLI.
- Tests never need a running herdr. A fake herdr is a shell script written
  to a temp dir and passed as `HERDR_BIN_PATH`. It appends its argv to a log
  file and prints canned JSON from `tests/fixtures/herdr/`. The canned JSON
  has the shape of herdr 0.9.1's output, with only the fields knapp reads
  and neutral values.
- A step is done when `make check` passes, its CLI commands give the expected
  output on the fixtures, and, from step 3 on, it works in a real herdr
  session through `herdr plugin link .`.

## Data model

`parse.rs` produces one `Parsed` per note. The cache stores it. `index.rs`
builds everything else on load.

```rust
pub struct Parsed {
    pub links: Vec<Link>,
    pub tags: Vec<Tag>,               // name, line
    pub headings: Vec<Heading>,       // level, text, line
    pub blocks: Vec<String>,          // `^id` block ids, as written
    pub frontmatter: Vec<(String, Value)>,
}

pub struct Link {
    pub kind: LinkKind,               // Link, Embed, Property
    pub markdown: bool,               // `[text](path)` form
    pub written: String,              // source text of the link, for output
    pub target: String,               // path part, decoded, `\` stripped
    pub fragment: Option<String>,     // after the first `#`, without it
    pub alias: Option<String>,
    pub span: Range<usize>,           // byte offsets in the file
    pub line: u32,                    // 1-based
}

pub enum Value { Str(String), List(Vec<String>), Date(String), Bool(bool), Raw(String) }

pub struct Index {
    pub root: PathBuf,
    pub files: Vec<File>,                     // FileId is the position
    by_path: HashMap<String, FileId>,         // key(rel path), extension kept
    by_name: HashMap<String, Vec<FileId>>,    // key(file name), extension kept
    pub forward: Vec<Vec<Resolved>>,          // per note, parallel to Parsed.links
    pub back: Vec<Vec<(FileId, usize)>>,      // (source note, link position)
}

pub enum Resolved {
    File { id: FileId, fragment: Fragment },
    Ambiguous { pick: FileId, others: Vec<FileId>, fragment: Fragment },
    Unresolved,
}

pub enum Fragment { None, Found, Missing }
```

`key(s)` is NFC, then `str::to_lowercase`. `File` holds the rel path, kind
(note or attachment), whether Obsidian's Excluded files cover it, mtime in
nanoseconds, size, and `Option<Parsed>` (`None` for attachments, excluded
files, and non-UTF-8 notes).

## Step 1: scan, parse, index, first CLI commands

Dependencies for this step: `pulldown-cmark`, `unicode-normalization`,
`serde`, `serde_json`, `toml`.

### config.rs

The config file as the plan describes it, with `deny_unknown_fields`, so a
top-level key written after a `[[root]]` fails with its name instead of
landing in that root. `pick_root` implements the CLI's root choice. Tests
set `XDG_CONFIG_HOME` to an empty directory so a user's own config cannot
change their results.

### scan.rs

- Recursive `read_dir`. Skip names starting with `.`. An `exclude` entry
  ending in `/` matches a directory and everything under it; any other entry
  matches one file. Both are relative to the root.
- Use `symlink_metadata`. Symlinked directories are skipped; symlinked files
  are indexed under the link's path.
- `.md` is matched case-insensitively. Everything else is an attachment.
- If `<root>/.obsidian/app.json` exists, read `userIgnoreFilters` (a JSON
  array of strings, or `null`). A trimmed entry of the form `/.../` is a
  regex: skip it and keep it for `index --stats`. Any other entry excludes
  every file whose rel path starts with it, compared case-insensitively.
  Excluded files stay in the file list, marked, and are not parsed.
- Return entries sorted by rel path, so output order is stable.

### parse.rs

pulldown-cmark 0.13 with `ENABLE_WIKILINKS`, `ENABLE_TABLES`, and
`ENABLE_YAML_STYLE_METADATA_BLOCKS`, read through `into_offset_iter()`. What
pulldown-cmark 0.13.4 does with Obsidian syntax:

| Input | Event |
|---|---|
| `[[a]]`, `[[a#h]]`, `[[a#^b]]` | `Link`, `LinkType::WikiLink`, `dest_url` holds `a#h` or `a#^b` unsplit |
| `[[a\|x]]` | `WikiLink { has_pothole: true }`, `dest_url` is `a` |
| `[[a\|x]]` escaped in a table | `dest_url` is `a\`, with the backslash |
| `![[a]]` | `Image`, `LinkType::WikiLink` |
| `` `[[a]]` `` and fenced blocks | `Code` / `CodeBlock`, no link |
| `%%[[a]]%%` | a normal link; comments are not recognized |
| `---` frontmatter | `MetadataBlock(YamlStyle)` with its byte range |

So `parse` runs two passes over the unchanged text:

1. Collect the byte ranges of `Code` and `CodeBlock`, then the comment
   ranges: `%%` marks outside code, paired in order. An unpaired `%%` runs
   to the end of the file. Comments are not blanked out, since that changes
   block structure: a line starting with `%%x%%` would become four spaces
   and an indented code block.
2. Walk the events, dropping any link, tag, heading, or block id that starts
   inside a comment:
   - Wiki `Link` / `Image`: strip one trailing `\` from `dest_url`, then
     split at the first `#`. The alias is the source text between `|` and
     `]]`. An empty target (`[[#h]]`) links to the same note.
   - `Inline` or `Reference` links and images whose destination has no
     scheme (`://`, `mailto:`): Markdown links. Percent-decode, then split at
     the first `#`.
   - Headings: collect the text events inside `Heading`.
   - Block ids: a paragraph or list item whose text ends in a space and `^id`, with
     `id` made of ASCII letters, digits, and `-`.
   - Tags: merge adjacent `Text` events, skipping text inside links, then
     scan for `#` at the start or after whitespace, followed by letters,
     digits, `_`, `-`, `/`, with at least one non-digit.
   - Frontmatter: the `MetadataBlock` range goes to a hand-written subset
     parser: `key: value`, `key: [a, b]`, and `key:` followed by `- a` lines.
     Quotes around a value are removed. `true` and `false` are `Bool`,
     `YYYY-MM-DD` values are `Date`, and any other shape is `Raw` text.
     `tags` accepts a list or a comma-separated string, and each value
     counts as a tag. A `Str` or `List` item that is exactly `[[...]]` or
     `[text](...)` is also a `Property` link, with its span inside the
     quotes.
3. `written` is the source text of the span. Line numbers come from a
   table of line start offsets, searched with `partition_point`.

### index.rs: resolution

`resolve(link, source) -> Resolved` ports Obsidian 1.14.2's
`getLinkpathDest`. `source_dir` is the source note's folder, lowercased,
with no trailing `/`. All comparisons are on `key()` strings:

```text
if target is empty: File(source)
o = key(target); name = last segment of o
cands = by_name[name] if name contains "." else none
if none: o = key(target + ".md"); name = last segment of o; cands = by_name[name]
if none: Unresolved
if name == o and one candidate: that candidate
if o starts with "./" or "../":
    join o to source_dir ("./../" drops its "./"; each "../" drops one folder)
    a candidate whose path == o: that candidate
drop one leading "/" from o
a candidate whose path == o: that candidate
if target starts with "/": Unresolved
keep candidates whose path ends with o (plain string test)
order: paths starting with source_dir (plain string test) first,
       then the rest; each group by length, then by path
one kept: File; several: Ambiguous with the first as pick; none: Unresolved
```

The two plain string tests are Obsidian's: `ab/same.md` ends with
`b/same.md`, and `notes2/x.md` starts with `notes`. Keep them, so
resolution matches Obsidian on the same vault.

Markdown links: join the decoded target to the source note's folder and
normalize `.` and `..` without touching the filesystem. A path that climbs
above the root is `Unresolved`. A path in `by_path` resolves to it.
Otherwise run the wikilink algorithm on the decoded target.

Fragments, following Obsidian's `resolveSubpath`: split the fragment on `#`
and drop empty parts. One part starting with `^` is a block id, matched
case-insensitively against `blocks`. Otherwise walk the headings in order,
matching part `g` against a heading deeper than the last match; all parts
matched is `Found`. Headings and parts compare after `norm()`: replace each
of `` !"#$%&()*+,.:;<=>?@^`{|}~/[]\ `` and CR/LF with a space, collapse
whitespace, trim, lowercase. A Markdown link's part also matches a
heading's GitHub slug: lowercase, drop characters other than letters,
digits, `-`, `_`, and space, then spaces to `-`.

`back` is rebuilt from `forward` in one pass, using the pick for ambiguous
links. A note's links to itself are left out, and so are links from excluded
files.

### CLI

A file argument is a path relative to the current directory, or absolute,
and must be inside the root. `links` needs a note; `backlinks` takes any
file, attachments included. A file outside the root, a missing file, or an
attachment given to `links` is an error (exit 1). Otherwise the commands exit
0 whatever they find. Output is tab-separated, one record per line, no
header:

```text
knapp links FILE       LINE  STATE  FRAGMENT  WRITTEN  TARGET
knapp backlinks FILE   SOURCE:LINE  LINE-TEXT
knapp unresolved       COUNT  STATE  TARGET
```

- `links`: one record per link in file order, frontmatter links first.
  `STATE` is `resolved`, `ambiguous`, or `unresolved`. `FRAGMENT` is `-`
  (no fragment, or unresolved), `ok`, or `missing`. `TARGET` is the resolved
  path; for ambiguous, the pick then the other candidates, joined with `,`;
  for unresolved, `-`.
- `backlinks`: one record per source line that links to FILE, sorted by
  source path then line. `LINE-TEXT` is that line, trimmed.
- `unresolved`: unresolved and ambiguous links grouped by state and
  `key(target)` without `.md`. `TARGET` is the target as first written.
  Sorted by count descending, unresolved before ambiguous, then target.
  `--json` prints
  `[{"target", "state", "count", "sources": [{"path", "line"}], "candidates"}]`.

### Fixtures and tests

The fixtures are in `fixtures/`, and `fixtures/README.md` lists what each
one covers. `scripts/expected.py` writes `tests/expected/`: it ports the
resolution above to Python and takes each note's links from a hand-written
list, so the expected output does not come from knapp itself. `make
expected` regenerates it; `make expected-check`, part of `make check` and
CI, fails when it is stale. `tests/expected/` holds the expected output for
every note:
`<fixture>.links.<file>.txt`, `<fixture>.backlinks.<file>.txt`, and
`<fixture>.unresolved.txt`, with `/` in the file path written as `_`. A
note with no backlinks has no backlinks file, and its expected output is
empty.

Tests:

- `tests/parse.rs`: each syntax gives the exact `span`, `target`,
  `fragment`, `alias`, and `line`, and the tag and block lists in
  `fixtures/README.md`.
- `tests/index.rs`: resolution states on every fixture, backlink inversion,
  self-links, excluded files (from a temp copy of `vault-basic` with an
  `app.json`), and NFC matching (an NFD-named file created at test time,
  since git may normalize a committed one).
- `tests/cli.rs`: for every file under `tests/expected/`, run the command on
  the fixture and compare stdout exactly.

## Step 2: cache, stat sweep, watcher

Dependencies for this step: `notify` 8, `sha2`.

### Cache (index.rs)

```rust
pub struct LoadStats {
    pub reused: usize,        // parses taken from the cache
    pub parsed: usize,
    pub stale: bool,          // the cache on disk no longer matches
    pub scan_ms: f64, pub cache_read_ms: f64, pub parse_ms: f64, pub resolve_ms: f64,
}

pub struct Change { pub added: Vec<String>, pub removed: Vec<String>, pub modified: Vec<String> }

pub fn cache_path(cache_dir: &Path, canonical_root: &Path) -> PathBuf;

impl Index {
    pub fn load(root: &Path, exclude: &[String], cache: Option<&Path>) -> Result<(Index, LoadStats), String>;
    pub fn refresh(&mut self, touched: &BTreeSet<String>) -> Result<Option<Change>, String>;
    pub fn save_cache(&self, path: &Path) -> Result<(), String>;
}
```

`load` never writes; the caller writes when `stale` is set.

- File: `{"format": N, "root": "...", "written_ns": T, "files": {"<rel>": {"kind", "mtime_ns", "size", "parsed"}}}`,
  compact JSON. `const CACHE_FORMAT: u32` sits in `index.rs`; any change to
  `Parsed` or to what `parse` extracts bumps it.
- Name: hex SHA-256 (`sha2`) of the canonical root path. `sha2` 0.11 has
  no hex formatter; format each byte with `{:02x}`.
- Directory: `cache_dir()` in `config.rs` returns the plugin state dir, else
  `$XDG_CACHE_HOME/knapp`, else `~/.cache/knapp`, and the file goes under
  `index/`.
- Reuse: a cached parse is used when the file is still a note that needs
  parsing (not excluded), mtime and size match, and
  `mtime_ns < written_ns - 2 s`. Everything else is parsed.
- A missing file, bad JSON, a different `format`, or a different `root`
  means an empty cache. None of these is an error.
- Write to `<name>.<pid>.tmp` in the same directory, then rename, so a CLI
  run and a pane can write without corrupting each other. A failed write
  prints `knapp: cache not written: <reason>` once and carries on.
- CLI commands load with the cache and write it when `stale` is set: a file
  was parsed, added, removed, or changed kind or exclusion, or there was no
  usable cache.

### Watcher (watch.rs)

`notify` 8's `recommended_watcher` (FSEvents on macOS, inotify on Linux),
recursive on the canonical root. On macOS, events carry canonical paths
(`/private/var/...`), a rename arrives as two unpaired `Modify(Name(Any))`
events, and the watched folder itself gets an event right after the watch
starts.

- A thread receives events and keeps the touched rel paths, dropping any
  with a component starting with `.` and any inside the cache directory
  (which may sit under the root). Access events are dropped too, except a
  close after writing: on Linux, notify watches `IN_OPEN`, so every folder
  a refresh opens would otherwise start the next refresh.
- It sends a batch after 150 ms without events, or 1 s after the first
  event of a continuous stream.
- `Index::refresh` runs the sweep again, reparses the touched notes plus any
  whose mtime or size changed, drops removed files, and re-resolves
  everything. It returns `None` when nothing changed, else a `Change` with
  the added, removed, and modified paths. Modified means any change to a
  file's mtime, size, kind, or exclusion, or a changed parse: a prose-only
  edit changes no links, but the pane shows the text and Recent the mtime.
- `FileId`s change on every refresh. Anything held across a refresh (the
  pane's open note, its history) holds rel paths.
- The pane does not write the cache after each refresh, only after its first
  load and on quit. After a crash, the next start's sweep catches up.

### `knapp index`

- Plain: load (writing the cache), print
  `<notes> notes, <attachments> attachments, <links> links`.
- `--rebuild`: delete this root's cache file first.
- `--stats`: `key<TAB>value` lines: `notes`, `attachments`, `excluded`,
  `links.resolved`, `links.ambiguous`, `links.unresolved`, `tags` (distinct),
  `cache.reused`, `cache.parsed`, `ms.scan`, `ms.cache_read`, `ms.parse`,
  `ms.resolve`, `ms.cache_write`, and one `skipped_filter` line per skipped
  `/regex/` filter.
- `--watch`: after the load, print one line per `Change`
  (`+N -N ~N <ms> ms`) until killed.

### Tests

- `tests/cli.rs` sets `XDG_CACHE_HOME` to a temp directory and removes
  `HERDR_PLUGIN_STATE_DIR`, so tests never touch a user's cache. The
  fixture test then covers cached loads: every run after the first per
  fixture reads the cache.
- `tests/common/mod.rs` makes temp copies of fixtures with mtimes a minute
  old, so the two-second rule does not force reparsing in tests.
- `tests/cache.rs`: a second load reuses every parse and gives identical
  `forward` tables; a wrong `format`, corrupt JSON, and an unwritable cache
  directory each fall back to parsing; rewriting a file with the same size
  within the same second is seen (the two-second rule); deleting a file
  drops it; `--rebuild` parses everything.
- `tests/watch.rs`: reading notes and folders (and refreshing, which opens
  every folder) produces no batch; on Linux this fails without the access
  filter. In a temp copy of `vault-basic`, start the watcher, then
  create, edit, rename, and delete notes, and wait (up to 5 s each) for the
  `Change` and the resulting state flip. In a temp copy, moving `alpha.md`
  to `x/alpha.md` keeps `[[alpha]]` resolved, and adding `y/alpha.md` makes
  it ambiguous with `x/alpha.md` as the pick. Writes under `.obsidian/`
  and in the cache directory produce no batch; a control run without the
  cache-dir ignore sees the same write.
- `tests/perf.rs`, `#[ignore]`: generate 5,000 notes (20 links each, one
  unresolved link and a block id per note, frontmatter tags) in a temp dir
  with a fixed seed, then assert the plan's targets for cold load, warm load,
  and one refresh. Run with `cargo test --release -- --ignored`.

## Step 3: the pane

Dependency for this step: `ratatui` 0.30 with default features off and
`crossterm`, `layout-cache`, and `underline-color` on. Use its
`ratatui::crossterm` re-export; there is no direct `crossterm` dependency.

### Layout of the code

- `src/render.rs`: note text to styled lines. Pure, no terminal.
- `src/tui/app.rs`: `App` state and key handling. No terminal.
- `src/tui/ui.rs`: drawing `App` into a ratatui `Frame`.
- `src/tui/mod.rs`: `run()`: terminal setup, the event loop, threads.
- `knapp pane [--root NAME|PATH]` calls `tui::run`.

### Start and shutdown

- `ratatui::init()` sets up the terminal and installs a panic hook that
  restores it; `ratatui::restore()` on the way out. Mouse capture is on while
  running and off before restore.
- Root: `--root`, else `KNAPP_CWD`, else `herdr::workspace_dir`: the cwd
  of the workspace's agent from `herdr agent list` (the focused pane if it
  is an agent, else the focused agent, else the first), else `workspace_cwd`
  from `HERDR_PLUGIN_CONTEXT_JSON` unless it is inside herdr's plugin
  directory (the `plugins` ancestor of `HERDR_PLUGIN_CONFIG_DIR`). Then
  `Config::pick_root`. Outside herdr, the current directory. Under herdr
  with no workspace directory, the first configured root, else an error.
  `workspace_cwd` is the focused pane's cwd, so with another plugin's pane
  focused it names that plugin's checkout.
- Load with the cache and write it when stale; write it again on quit. A
  root that fails to load shows the error in the pane; `q` still quits.
- The pane's cwd is the plugin root, never the notes root.

### Event loop

- One thread reads crossterm events, one forwards watcher batches. The
  batch thread owns the `Watch` and calls `next_batch()`: a `move` closure
  that names only `watch.batches` captures only that field, the watcher is
  dropped when the spawning scope ends, and no event ever arrives. Both send
  `AppEvent::{Key, Mouse, Resize, Batch}` into one channel. The main loop
  blocks on it, applies the event to `App`, and redraws.
- On `Batch`, `Index::refresh`; on `Some(change)`, drop every render cache
  (a new file can change link states in any note), keep the open note,
  selection, and history by rel path, and show `updated: +a -r ~m` on the
  status line. An open note that was removed shows `deleted: <path>` in the detail
  panel; history skips removed entries.

### App state

```rust
pub struct App {
    index: Index,
    mode: Mode,                      // Tree, Backlinks, Forward in this step
    focus: Focus,                    // List or Detail
    list: ListState,                 // selected row, offset
    expanded: BTreeSet<String>,      // open folders in Tree, by rel path
    open: Option<String>,            // rel path of the note in the detail panel
    history: Vec<Visit>, cursor: usize, // Visit { rel, scroll }
    scroll: usize,                   // detail panel, in rendered lines
    link: Option<usize>,             // selected LinkHit
    fold_frontmatter: bool,
    status: Option<String>,          // one-line message at the bottom
    help: bool,
}
```

- `tab` cycles the modes built so far; later steps add theirs.
- Tree rows: folders (folded by default), then files, each sorted by name
  without regard to case. Excluded files are not listed. `enter` on a folder folds or
  unfolds it; on a file it opens it.
- Backlinks rows: `source:line` and the trimmed linking line. `enter` opens
  the source scrolled to that line with that link selected.
- Forward rows: one per link, with state, as written, and target. `enter`
  follows it.
- Opening a note pushes a `Visit`; `[` / `ctrl-o` and `]` move through
  history, restoring scroll.
- Following: resolved or ambiguous goes to the target (the pick), scrolled
  to the heading or block when the fragment was found. Unresolved shows a
  detail page: `No note named <target>` and the notes that reference it,
  each followable. An attachment shows its path, kind, and size.
- Width under 80 columns: `narrow()`; only the focused panel is drawn.

### render.rs

```rust
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub source_line: Vec<u32>,       // parallel to lines
    pub hits: Vec<LinkHit>,
}
pub struct LinkHit { pub line: usize, pub cols: Range<u16>, pub link: usize } // link: index into Parsed::links

pub fn render(text: &str, parsed: &Parsed, states: &[Resolved], width: u16, fold_frontmatter: bool, theme: Theme) -> Rendered;
```

- pulldown-cmark with the parse options plus `ENABLE_TASKLISTS` and
  `ENABLE_STRIKETHROUGH`, over the same text, so link offsets match
  `Parsed::links`: a rendered link is matched to its `Link` by
  `span.start`. A link with no match (an external URL) renders as text
  with its URL, and is not in `hits`.
- Wrap by display width at spaces; a word longer than the width is broken.
  Code block lines and table cells are cut with `…`.
- Headings: bold, with dim `#` markers, blank line before. Lists: `•` and
  numbers, two spaces per level, `[ ]` / `[x]` for tasks. Quotes: a `│`
  and a space before each line. A quote whose first line starts `[!type]` is a callout: the
  marker becomes `▌ Type:` and the title line ends at the first line break.
  pulldown-cmark splits `[!type]` across text events, so the marker is read
  across them. Rules: a `─` line. Inline and fenced code:
  a distinct color, or reverse with `NO_COLOR`.
- Links: alias, link text, or target; underlined. Resolved in one color,
  ambiguous in another, unresolved dim. Wiki embeds of notes render as
  `↳ <target>`; image embeds as `[image: <name>]` until step 7.
- Frontmatter: folded to `▸ frontmatter (<n> keys)`; unfolded as
  `key  value` rows. Property links are hits like any other.
- Comment ranges (`%%...%%`) are not rendered.
- Cache `Rendered` by (rel, width, fold state), cleared on refresh for
  changed paths.

### Manifest and a real session

Add the `notes` pane from the plan to `herdr-plugin.toml` in this step.
The step closes after this runs in Herdr:

1. `cargo build --release`, then `herdr plugin link .` (link does not
   build).
2. `herdr plugin pane open --plugin shindakun.knapp --entrypoint notes`
   from a workspace whose directory holds notes: the pane opens on that
   root.
3. Keys from the plan's table, a resize, and the mouse.
4. Edit a note in another pane: the open note and its backlinks update.
5. `q`: the terminal is restored and the cache is written.

### Pane tests

- `tests/render.rs`: every block type at 30 columns, wrapping that keeps
  punctuation with its link, hits that point at the right links on
  `vault-basic/index.md`, frontmatter folded and open with a property link
  hit, link colors by state and with `NO_COLOR`, and `cut`.
- `tests/pane.rs`: `App` on `TestBackend` driven by key events: the summary
  and folded tree at start, folding a folder, following
  `[[alpha#Second Section]]` in a six-row terminal so the jump must scroll,
  back and forward, an unresolved link page, backlinks opening the source
  with its link selected, Forward rows with state, the narrow layout, help,
  a refresh that keeps the open note and then shows it deleted, and quit.
- `tests/herdr.rs`: agent choice, and `workspace_dir` refusing a plugin
  checkout in `workspace_cwd` in favor of the agent's directory.
- `tests/watch.rs`: a `Watch` handed to a reader thread inside a helper
  function keeps delivering batches. With the field-only capture this test
  fails with `Disconnected`.

## Step 4: editor, copy, search

No new dependencies.

### Effects

`App` stays free of terminal and process work. Keys that need it push an
`Effect`, and the event loop in `tui/mod.rs` carries it out:

```rust
pub enum Effect {
    Edit { path: PathBuf, line: u32 },
    Copy(String),
    Search { generation: u64, query: String },
}
impl App { pub fn take_effects(&mut self) -> Vec<Effect>; }
```

Tests assert on effects; the loop is checked in herdr.

### Editor

- `editor_command(editor: &str, path: &Path, line: u32) -> Vec<String>`:
  split on whitespace (`code -w` works; quoting is not supported). By the
  command's basename: `vi`, `vim`, `nvim`, `nano`, `emacs`, `micro`, `kak`
  get `+LINE FILE`; `hx` gets `FILE:LINE`; `code` gets `-g FILE:LINE`; any
  other editor gets `FILE`.
- Choice: config `editor`, `$VISUAL`, `$EDITOR`, then `vi`.
- The line: the selected hit's source line, else the source line of the top
  visible rendered line.
- The input thread must not read the terminal while the editor runs, or it
  takes the editor's keystrokes. It polls with a 50 ms timeout and checks a
  shared `paused` flag between polls; when paused it sets an `idle` flag and
  sleeps. The loop sets `paused`, waits for `idle`, then suspends:
  `DisableMouseCapture`, `LeaveAlternateScreen`, `disable_raw_mode`, show
  the cursor; runs the editor with `Command::status` in the root; then
  `enable_raw_mode`, `EnterAlternateScreen`, `EnableMouseCapture`,
  `terminal.clear()`, and clears `paused`. It does not call
  `ratatui::init()` again, which would stack another panic hook.
- A failed spawn shows `editor: <error>` on the status line.

### Copy

- `editor.rs` holds `choose`, `command`, `base64`, and
  `osc52(text) -> String`: `ESC ] 52 ; c ; <base64> BEL`, written straight
  to stdout and flushed; ratatui's buffer is untouched. The base64 encoder
  is a few lines; no crate. Herdr accepts up to 192 KiB.
- `y`: the note's absolute path. `Y`: the shortest trailing part of its
  path, without `.md`, that `Index::wikilink` from a note at the root
  returns alone; else the full path. On an unresolved page, the target.
  The status line says what was copied.

### Search

- `Mode::Search` joins the `tab` cycle after Forward. `/` switches to it and
  opens the query line; while it is open, keys edit the query (characters,
  `backspace`, `ctrl-u` to clear), and `enter` or `esc` closes it. Each
  change pushes `Effect::Search` with a new generation.
- `search.rs`:
  `search(root, query, exclude, notes, rg, cancel, sink)` streams `Match`
  values (`rel`, `line`, `text`, byte `ranges`) to `sink` until it returns
  false, `cancel` is set, or 500 results. With `rg` on `PATH`:
  `rg --json --fixed-strings --smart-case --no-ignore --iglob '*.md' --glob '!<dir>/**' -- <query> .`
  for each `exclude` folder, in the root, with stdin set to null. Without
  a path argument, `rg` searches stdin whenever stdin is not a terminal and
  waits forever. Read `match` records: `path.text`
  (skip `path.bytes`, which is a non-UTF-8 name), `line_number`,
  `lines.text`, and `submatches[].start..end` (byte offsets into the line).
  Without `rg`, read each indexed note and match lines, lowercasing both
  sides when the query has no uppercase letter.
- The loop runs one search thread at a time. A new query kills the old `rg`
  child and bumps the generation; results from an old generation are
  dropped. Results arrive as `AppEvent::Results(generation, rows)` in
  batches.
- The app keeps only results whose path is an indexed, visible note, and
  highlights the match ranges.

### Tests

- `tests/editor.rs`: `editor_command` for each editor family, a command
  with arguments, and the fallback order (config, `VISUAL`, `EDITOR`, `vi`).
- `tests/pane.rs`: `o` pushes `Edit` with the selected link's line, and
  with the top visible line when no link is selected; `o` on an attachment
  sets a status and pushes nothing; `y` and `Y` push the right `Copy`;
  typing `/alp` pushes three `Search` effects with rising generations, and
  results from an old generation are ignored.
- `tests/search.rs`: both backends on `vault-basic` and `vault-syntax`
  give the same results: smart case, a match inside code, a file outside
  the index (a dot folder, an excluded folder, a `.gitignore`d file in a
  temp copy) never listed. The `rg` test is skipped when `rg` is not on
  `PATH`.
- `osc52` against a known base64 string.

### In herdr

Link, open the pane, and:

1. With `editor = "vi"` in the config: `o` on a note opens vi at the line;
   `:q` returns to the pane with the screen redrawn and keys working. Keys
   typed in vi reach vi, not the pane.
2. Edit and save in vi: the note updates in the pane.
3. `y`, then paste in another pane: the path. `Y`: the wikilink.
4. `/` and a word: results fill in while typing; `enter` on one opens the
   note at that line.

## Step 5: tags, unresolved, orphans, recent

No new dependencies.

### Index queries (index.rs)

```rust
pub struct TagNode { pub name: String, pub shown: String, pub notes: Vec<FileId>, pub children: Vec<TagNode> }
impl Index {
    pub fn tags(&self) -> Vec<TagNode>;          // tree, sorted by name, case-insensitive
    pub fn orphans(&self) -> Vec<FileId>;        // notes, not excluded, empty `back`
    pub fn unresolved(&self) -> Vec<Target>;     // the groups `knapp unresolved` prints
}
```

- A tag's key is its name lowercased; `shown` is the spelling with the
  most occurrences, ties to the first in path order (Obsidian's
  `getTags` keeps the most frequent spelling). `notes` on a node are the notes tagged exactly that tag; a node's
  count is the distinct notes in it and every node under it.
- Move the grouping in `cli::unresolved` into `Index::unresolved`, so the
  CLI and the Unresolved mode share it: `Target { ambiguous, key, shown,
  sources: Vec<Source>, candidates: Vec<FileId> }`, sorted as the CLI
  prints. `Source { file, line, pick }` records where that link goes: an
  ambiguous name has no single pick, since Obsidian prefers candidates
  under the linking note's folder. `candidates` are sorted by path.
  `unresolved --json` gives each source's `goes_to`.

### Modes (tui/app.rs)

- `Mode::ALL` gains Tags, Unresolved, Orphans, and Recent between Forward
  and Search.
- Tags: tag rows fold like folders (`expanded_tags`, by key). An unfolded
  tag lists its child tags, then its notes. Rows show the count after the
  name. `enter` on a note opens it.
- Unresolved: `count  target`, marked `✗` or `?`. `enter` opens
  `Page::Unresolved(target)` for an unresolved group and
  `Page::Ambiguous(key)` for an ambiguous one. The ambiguous page lists
  the candidates, then `Referenced from:` with `source:line → target` for
  each link, all followable.
- Orphans: paths, sorted. `enter` opens.
- Recent: every visible note, newest first, as `age  path`. Age is the
  largest whole unit of the time since mtime: `42s`, `5m`, `3h`, `2d`,
  `6w`, `14mo`, `2y`. No clock or timezone crate is needed.
- All four rebuild on refresh, like the Tree.

### Header (tui/ui.rs)

Measure the full mode list; when it and the root label do not fit the
width, show `◂ Mode ▸` for the current mode only.

### CLI

- `knapp orphans [--root]`: one root-relative path per line, sorted.
- `knapp tags [--root]`: `COUNT<TAB>TAG` per tag, every level, as shown,
  sorted by name without regard to case, parents before children.
- `cli::unresolved` prints from `Index::unresolved`; its output is
  unchanged.

### Expected output

`scripts/expected.py` gains `TAGS`, each note's tags by hand, in the style
of `LINKS`, and writes `<fixture>.tags.txt` from it and
`<fixture>.orphans.txt` from its own backlinks. `tests/cli.rs` runs
`orphans` and `tags` on every fixture. These land in the same change, since
an expected file with no test is an error in `tests/cli.rs`.

### Tests

- `tests/cli.rs`: the new expected files.
- `tests/index.rs`: tag case folding and most-used spelling, parent counts
  with a note counted once across two child tags, orphans ignoring
  self-links and excluded files.
- `tests/pane.rs`: tab order, unfolding a tag and opening a note from it,
  the ambiguous page on `vault-ambiguous` (each source's own pick), Recent
  ordering after touching a file, the one-mode header at 40 columns, and
  an open note showing a prose-only edit.
- `tests/cache.rs`: a prose-only edit is reported as modified.
- `age()` for each unit boundary.

### In herdr

Link and open the pane on a scratch copy of a fixture:

1. `tab` through every mode; the header fits the split.
2. Tags: unfold a nested tag and open a note from it.
3. Unresolved: open an ambiguous target and follow a candidate.
4. Edit a note in another pane: it moves to the top of Recent.

## Step 6: send

No new dependencies.

### send.rs

Pure functions, tested without herdr:

```rust
pub fn check(root: &Path, allow: &[String], notes: &[PathBuf]) -> Result<(), Refusal>;
pub fn clean(text: &str) -> String;       // control characters out, \r\n to \n
pub fn fence(request: &str, notes: &[(String, String)], suffix: &str) -> String; // (rel, text)
pub fn suffix() -> Result<String, String>; // 8 hex chars from 4 bytes of /dev/urandom
pub fn valid_pane_id(id: &str) -> bool;    // w[A-Za-z0-9]+:p[A-Za-z0-9]+
```

- `check` canonicalizes the root joined to each prefix and each note. A
  prefix that fails to canonicalize allows nothing. A note passes when some
  canonical prefix equals it or is a whole-component ancestor of it
  (`Path::starts_with`, which compares components). `Refusal` lists every
  refused path.
- `clean` drops C0 controls except `\t` and `\n`, DEL, and C1 (U+0080 to
  U+009F), after turning `\r\n` into `\n`. It runs on the request, every
  path, and every note text. Herdr (0.9.1, `src/pane.rs` `paste_payload`)
  wraps paste text in `ESC[200~ … ESC[201~` without changing it on macOS
  and Linux. Sent unclean to Claude Code, a note with `ESC[201~` and a
  newline ends the paste, and the lines after it arrive as a separate,
  unfenced user message.
- `fence` writes the request (if any) and a blank line, the preamble line,
  then one `<knapp-note-SUFFIX path="...">` element per note.
- `prompt(request, notes, max)` cleans the notes, then draws a suffix that
  no cleaned note contains as `knapp-note-SUFFIX`. The check must run on
  the cleaned text: cleaning removes characters, so `knapp-note-\x01…` in a
  raw note becomes the tag. `prompt_with` and `unused_suffix_from` take the
  draws as a closure so tests can make them known.
- The size check runs on the final text.
- `/dev/urandom` read failures refuse the send; there is no fallback to a
  predictable suffix.

### Picking the agent

- `herdr::agents()` (step 3) and `pick_agent`. Keep the agents whose
  `workspace_id` is the context's `workspace_id`, and drop any whose
  `pane_id` fails `valid_pane_id`.
- One agent: straight to the send line. Several: a picker overlay (`j`/`k`,
  `enter`, `esc`) of `agent  status  pane  cwd`, preselecting the pane id in
  `HERDR_PLUGIN_STATE_DIR/last-agent`. None: `no agent in this workspace`.
- The agent list is read when `s` is pressed, not at start: agents come and
  go.

### The send line

- `App` state: `send: Option<SendDraft { agent, notes: Vec<String>, request: String }>`.
  `s` and `S` run `check` first; a refusal sets the status
  (`not sent: outside send_allow: a.md, b.md`) and opens nothing.
- While open, keys edit `request` (as in the query line); `enter` pushes
  `Effect::Send { pane, text }` with the fenced text; `esc` closes it.
  The line shows `to <agent> <pane> · <n> notes · <size> ›` and the request.
- `S`'s notes: the open note, then the distinct sources of its backlinks in
  path order.
- `App` asks for agents with `Effect::ListAgents` and gets them through
  `App::agents(result, last)`, since it does no process work itself.
- The loop runs `herdr agent prompt <pane> <text>` (`herdr::prompt`) in a
  thread and sends back `AppEvent::Sent(Result<String, String>)`:
  `sent to <agent>` or herdr's error (such as `agent_blocked`) on the
  status line; `herdr::error_message` turns herdr's JSON error into
  `code: message`. On success it writes `last-agent`.

### Tests

- `tests/send.rs`, each case in a temp dir built at test time: `..` in the
  path, a symlink out of the prefix, `notes-private/` against `notes/`,
  wrong case (skipped when the temp dir's filesystem is case-sensitive, as
  on Linux CI), a prefix that names a file, a missing prefix, `S` with one
  outside backlink, an oversize send, a note containing `</knapp-note-`, a
  note containing `ESC[201~` and `\r\n` (gone, and newlines kept), a path
  with `&<>"`, pane ids that pass and fail, and a note holding the first
  drawn tag split by a control character (the draw is skipped).
- The narrow header keeps `send:` at 36 columns.
- `tests/pane.rs`: `s` with no `send_allow` refuses with the paths; with
  one agent, the send line opens with the target and size; typing a request
  and `enter` push one `Effect::Send` whose text starts with the request and
  holds the fence; `esc` pushes nothing; `S` lists the backlinks.
- A fake herdr (a shell script as `HERDR_BIN_PATH`, printing
  `tests/fixtures/herdr/agent_list.json` and logging argv) checks that a
  refusal never runs `agent prompt` and an allowed send passes the text as
  one argument.

### In herdr

Send only to a scratch agent started for the test in its own split, never
to an agent doing other work.

1. A config with `send_allow` for a scratch root; open a note inside and one
   outside the prefix. Outside: refused, and nothing reaches the agent.
2. Inside: the send line shows the scratch agent; type a request, `enter`.
   The agent receives one message: the request, the preamble, and the
   fenced note.
3. A note containing an `ESC[201~` sequence and a line after it: the agent
   receives it as one paste, with no extra submission.

## Step 7: graph and images

Two changes, each checked in herdr and committed on its own: the graph
first, then PNG embeds.

Dependency: `tiny-skia` (default features, for `png-format`), for
antialiased edges and dots and PNG encoding. Labels are terminal text, so
knapp embeds no font.

### Pane graphics (herdr 0.9.1)

- Socket client in `herdr.rs`: `UnixStream` to `HERDR_SOCKET_PATH`, one
  JSON request per line, one JSON reply per line. Calls:
  - `pane.graphics.info {pane_id}`: `cell_width_px`, `cell_height_px`,
    `pane_visible`, `max_layers_per_pane` (16).
  - `pane.graphics.set {pane_id, layer_id, z_index, format: "png", image_width, image_height, data_base64, placement: {viewport_col, viewport_row, grid_cols, grid_rows}}`
  - `pane.graphics.clear {pane_id, layer_id}`
- `pane_id` is `HERDR_PANE_ID`; a pane without it (outside herdr, or a
  popup) has no graphics. An error reply (`feature_disabled`) turns
  graphics off for the session.
- Herdr passes `z_index` straight to the kitty placement's `z`
  (`src/kitty_graphics.rs`), and the kitty protocol draws a negative `z`
  under text. knapp uses `z_index` −1, so text written in the cells shows
  over the image.
- `viewport_col` and `viewport_row` are relative to the pane and may be
  negative; herdr clips the placement. `grid_cols` and `grid_rows` size the
  image in cells; the terminal scales it.
- Query `pane.graphics.info` at start and on every resize event (the cell
  size follows the font size).
- `herdr plugin pane open` without `--target-pane` opens in the focused
  workspace, which may not be the one the command ran from. A split needs
  `--target-pane <id>`; `--workspace` alone is refused for a split.

### Effects

`App` never touches the socket. Each draw leaves `app.layers`: the
layers the screen should have, as
`Layer { name, key, content, band, at }`. `content` is the graph canvas or
an image file; `band` is the rows of it inside the note area. Herdr clips
only to the pane, so `ui::draw_detail` cuts each layer to the note area and
places the band at its first visible row; the loop draws the canvas for
those rows (`graph::canvas(spec, px, band)`) or cuts the image
(`graph::crop_rows` on the decoded file, decoded once and cached). A layer
shown whole is sent as the file's own bytes. A picture that cannot be
decoded is left out; only herdr refusing a call turns graphics off. The loop compares them with what it last sent and sends `set` for
new or moved layers and `clear` for gone ones; unchanged layers are not
sent again. Content (the PNG) is built by the loop and cached by key, since
`App` has no cell size in pixels. The help overlay and the picker empty
`app.placements`.

### Graph (graph.rs)

```rust
pub struct Node { pub id: FileId, pub hop: u32, pub parent: Option<FileId>, pub mark: Mark }
pub enum Mark { Root, Out, In, Both }
pub struct Local { pub nodes: Vec<Node>, pub edges: Vec<(FileId, FileId)>, pub left_out: usize }

pub fn local(index: &Index, root: FileId, hops: u32, cap: Option<usize>) -> Local;
pub fn tree(index: &Index, local: &Local) -> Vec<String>;      // the indented lines
pub fn dot(index: &Index, local: &Local) -> String;
pub fn layout(index: &Index, local: &Local, cols: u16, rows: u16) -> Vec<(u16, u16)>; // cell per node
pub fn canvas(local: &Local, cells: &[(u16, u16)], cols: u16, rows: u16, cell_px: (u32, u32)) -> Vec<u8>; // PNG
```

- `local`: breadth-first from `root` over links in both directions, using
  `forward` targets (resolved and the pick of ambiguous links) and `back`.
  A node's `parent` is the node that first reached it, children visited in
  path order. `edges` are forward links between included nodes, each once.
  Self-links are dropped. The cap cuts the farthest hop first, fewest links
  first within it.
- `tree`: the root's path, then each node indented two spaces per hop
  under its parent with its mark: `-> alpha.md` at hop 1 is indented two spaces. `left_out > 0` adds
  `(N more not shown)`.
- `dot`: `digraph knapp {`, one quoted node per line sorted by path, then
  one `"a" -> "b";` per edge sorted, then `}`. `"` and `\` in paths are
  escaped.
- `layout`: Fruchterman-Reingold, 300 iterations, starting positions from a
  hash of each path, the root pinned at the center; positions rounded to
  cells, nudged so no two nodes share a cell. The same input always gives
  the same layout.
- `canvas`: a `tiny-skia` pixmap of `cols × cell_width` by
  `rows × cell_height`, transparent; edges as 1.5 px lines, dots at each
  node's cell centre with radius from `ln(inbound + 1)`, the root in
  another color. `encode_png`.

### Graph page (tui)

- `g` opens `Page::Graph(rel)` for the open note; history treats it like
  any page.
- The detail is built by the app: the canvas rows (when graphics are on),
  with each node's name written at its cell as a link hit, then a blank
  line, then the tree with each name a hit. The canvas takes 60% of the
  detail height, at least 10 rows, when graphics are on.
- Placement: layer `graph`, over the canvas rows, moving with scroll.
- Labels: right of the dot when the name fits, else left of it (ending
  next to the dot), else cut with `…` on the roomier side. A label that
  would overlap another is left off the canvas; the tree lists it.

### `knapp graph`

`knapp graph FILE [--root] [--hops N] [--dot]`: the tree (no cap), or
Graphviz. `scripts/expected.py` computes the tree for every fixture note
from its own links and backlinks, and `tests/cli.rs` compares.

### Images (second change)

- In `render.rs`, a `![[x.png]]` or `![](x.png)` that resolves to a PNG of
  at most 8 MB, with graphics on, reserves rows instead of the placeholder:
  width in cells is the panel width, capped at the image's own width in
  cells; rows keep the aspect ratio in pixels. The width and height come
  from the IHDR chunk (bytes 16 to 23), read once per file.
- `render_with_images` takes the slots by link index and reserves blank
  rows for each; `Rendered` gains `images: Vec<ImageSlot { line, rows,
  cols, link }>`, and the app maps each to an `ImageRef` with its path and
  a content key (path, mtime, size). `App::set_cell_px` drops every cached
  page when the cell size changes.
- Placement: layers `img-0` to `img-13` for the visible slots in order,
  content the file's bytes (format `png`). Slots past 14 keep the
  placeholder line.

### Tests

- `tests/graph.rs`: `local` on `vault-broken` (the three-note cycle, the
  orphan reaching `hub`), hop limits, the cap cutting the farthest hop, the
  tree text, `dot` escaping, `layout` twice with the same result and no
  shared cells, and `canvas` producing a PNG of the right size (decoded
  with `tiny_skia::Pixmap::decode_png`).
- `tests/cli.rs`: `graph` for every fixture note against `tests/expected/`.
- `tests/pane.rs`: `g` shows the tree, `n` and `enter` open a node,
  `app.layers` holds `graph` with graphics on and nothing with them off or
  with help open, and the layer's row follows scroll in a short pane.
- `tests/graphics.rs`: the socket calls against a fake socket server that
  records the request lines and answers `pane.graphics.info`, and errors
  coming back as `code: message`. Resending only changed layers is the
  event loop's `Sync`, checked in herdr.
- Images: the IHDR reader on `vault-basic/img.png`, reserved rows in
  `tests/render.rs`, an `img-0` layer for `index.md` with graphics and the
  placeholder without, the 8 MB cutoff, `crop_rows` taking the matching
  pixel rows, and a scrolled canvas cut rather than moved above the note
  area.

### In herdr

The screen cannot be captured from here, so the user looks:

1. `g` on a note with links: dots and edges under the names, the open note
   in the middle; `n` / `enter` open a node.
2. Resize the pane and scroll: the image follows.
3. `?` while the graph shows: the image goes; `esc`: it comes back.
4. A note with a PNG embed shows the image in place of the placeholder.
5. Scrolled so the image is partly above the note area, only its lower
   rows show, below the title.

## Step 8: actions, link handler, peek

No new dependencies.

### open-pane (`knapp open-pane`)

- Port herdr-rss `src/launch.rs`: `decide(pane_list_json, plugin_root,
  focused_tab)` returns open, focus, or close for a pane in the focused
  tab whose `label` is `Knapp` and whose `cwd` is the plugin root (a knapp
  pane's `pane list` entry has both). Only `w…:p…` pane ids reach an argv.
- Focus: `herdr plugin pane focus <id>`; close: `herdr plugin pane close
  <id>` (`herdr pane focus` takes a direction, not an id); open:
  `herdr plugin pane open --plugin shindakun.knapp --entrypoint notes
  --target-pane <focused_pane_id> --env KNAPP_CWD=<dir>`, with `<dir>` from
  `herdr::workspace_dir` (step 3). Without `--target-pane`, herdr opens in
  whatever workspace has focus.

### peek-selection (`knapp peek-selection`)

- `target(context) -> Result<(PathBuf, Option<String>), String>`:
  - `clicked_url`: `file://[host]/path[#fragment]`. Host empty,
    `localhost`, or `hostname`'s answer; anything else is refused.
    Percent-decode the path.
  - `selected_text`: first line, trimmed, then surrounding `[[…]]`,
    quotes, or backticks removed, and an `|alias` or `#fragment` split
    off. An absolute path; else relative to the workspace directory; else a
    wikilink target resolved from the root of each configured root in order
    (the first root that resolves it wins); else not found.
- Open: `plugin pane open --plugin shindakun.knapp --entrypoint peek --env
  KNAPP_NOTE=<path>` plus `--env KNAPP_FRAGMENT=<fragment>` when there is
  one. herdr's popup is modal: `ui_busy` when another is open.
- Failures (no match, a refused host) open the peek popup with
  `KNAPP_ERROR=<reason>`, which `knapp peek` shows until `q` or `esc`
  (`tui::show_message`). Herdr suppresses notifications for the active tab,
  so `herdr notification show knapp --body <reason>` is only the fallback
  for `ui_busy`, when the popup cannot open. The action exits 1 either
  way, so herdr's log has the reason.

### peek (`knapp peek`)

- Root: the configured root containing `KNAPP_NOTE`, else the nearest
  folder above it holding `.obsidian/` or `.git/`, else its folder.
- An `App` in peek mode: detail only (no list, `tab` does nothing), opened
  on the note and scrolled to `KNAPP_FRAGMENT` (heading or block). Keys:
  scroll, `n`, `N`, `enter` (follows inside the popup), `[` `]`, `o`, `y`,
  `Y`, `g`, and `q` / `esc` to close. `s` works; the popup still gets the
  underlying pane's context. The popup has no `HERDR_PANE_ID`, so there
  are no pane graphics: the graph is its tree and images are placeholders.

### Manifest

Add the `peek` pane, the `open` and `peek-selection` actions, and the link
handler, as in the plan. The link handler pattern is
`(?i)^file://[^?#]*\.md(#.*)?$`. The README gets both keybindings.

### Tests

- `tests/launch.rs`: `decide` on `pane list` JSON shaped like herdr 0.9.1's
  (open with none, focus when unfocused, close when focused, a knapp pane
  in another tab ignored, a malformed id refused).
- `tests/peek.rs`: URL decoding with and without a host, `%20`, a
  fragment, a foreign host refused; selected-text cleanup (`[[a|b]]`,
  quotes, backticks, several lines); resolution order; the peek root rule
  (a config root, `.obsidian/`, `.git/`, none).
- `tests/actions.rs`: the real binary with a fake herdr (a shell script as
  `HERDR_BIN_PATH`) that logs argv and answers `pane list`, `agent list`,
  and `plugin pane open` (including `ui_busy`): the focus and open argv;
  the peek argv from a clicked URL with a fragment and from a selected
  wikilink resolved through the config; the error popup for a missing note
  and a foreign host; a notification only for `ui_busy`.
- `tests/pane.rs`: peek mode draws no list, `tab` does nothing, and `esc`
  quits.

### In herdr

Test panes open with `--target-pane` next to the session's own pane. The
popup has no pane id, so `pane read` cannot see it; the user looks.

1. `herdr plugin action invoke shindakun.knapp.open` three times from a
   split: opens, focuses, closes.
2. In a scratch shell pane, print an OSC 8 link to a note
   (`printf '\e]8;;file:///tmp/…/a.md\e\\a.md\e]8;;\e\\\n'`) and
   activate it with the socket method `pane.link.activate {pane_id,
   viewport_row, col}` (not in the CLI): the peek popup opens on the note.
3. With `#heading` on the URL: the popup opens at that heading.
4. A link to a missing file: the popup shows the reason, and `q` closes it.

For step 3, the note needs text below the heading: a heading near the end
of a short note cannot scroll to the top, and the popup shows the end.

## Step 9: multiple roots

No new dependencies.

### Session (tui/session.rs)

The event loop holds a `Session` instead of one `App`:

```rust
pub struct Session {
    slots: Vec<Slot>,          // slot 0: the unconfigured start root, if any; 1..: config order
    active: usize,
}
struct Slot { root: Root, app: Option<App> }   // `app` loads on first switch

impl Session {
    pub fn active(&mut self) -> &mut App;
    pub fn switch(&mut self, n: usize, load: impl FnOnce(&Root) -> Result<App, String>) -> Result<bool, String>;
    pub fn app_mut(&mut self, slot: usize) -> Option<&mut App>;
}
```

- Slots: configured roots are `1..=N` in config order (at most 9 get a
  key). The start root is the configured one that contains the workspace
  directory, or else slot `0`, the workspace directory itself. With no
  slot 0, key `0` does nothing.
- `App` handles `0`..`9` by pushing `Effect::SwitchRoot(n)`; peek mode and
  open query or send lines ignore digits as switches (the lines take them
  as text).
- Switching keeps the old `App` whole (its index, history, list, folds,
  search, scroll). A new slot's `App` is loaded through the same
  `load_root` the pane starts with; a load failure sets the status line
  and stays on the current root.
- The root switched to takes the current `cell_px`, loaded or not: the
  cell size follows the terminal, and a root loaded before a resize would
  otherwise lay out images at the old size.

### Events carry their slot

Everything that comes back later is tagged with the slot that asked:
`AppEvent::Batch(slot, …)`, `Results(slot, generation, …)`,
`Agents(slot, …)`, `Sent(slot, …)`. The loop routes each to
`session.app_mut(slot)`, so a switch never delivers one root's search
results, agent list, or refresh to another (generations are per `App`
and can collide across roots).

### Watchers

One watcher per loaded slot, started when the slot first loads, each
sending `Batch(slot, …)`. An inactive root keeps refreshing, so switching
back shows it current. The cache for every loaded slot is written on quit.

### Header

With more than one slot, the root label is `<n> <name>`; the narrow
fallbacks shorten the name but keep the number. `?` lists the slots.

### Tests

- `tests/session.rs`: slot numbering with and without a slot 0; `switch`
  loads once and keeps state (open a note in root 1, switch to 2 and back:
  the note and history are still there); a failing load leaves the active
  root; `app_mut` routing.
- `tests/pane.rs`: digits push `SwitchRoot`; the query and send lines take
  digits as text; peek ignores them; the header shows `2 notes`.
- A send from root 2 is checked against root 2's `send_allow`.

### In herdr

With two configured roots, `2` and `1` switch, each keeps its open note,
and editing a file in the inactive root shows when switching back.

## Releasing

1. Date the version's section in `CHANGELOG.md`:
   `## X.Y.Z (YYYY-MM-DD)`. `release.sh` refuses an undated heading and
   takes the notes up to the next `##` heading or the end of the file.
2. Push `main` and let CI pass; `release.sh` requires `main` in sync with
   `origin`.
3. `scripts/release.sh X.Y.Z`: checks, version bump in `Cargo.toml`,
   `Cargo.lock`, and `herdr-plugin.toml`, commit, tag `vX.Y.Z`, push, and
   a GitHub release with the changelog section.
4. On a clean herdr: `herdr plugin install shindakun/herdr-knapp`, bind
   the keys from the README, open the pane, peek a link, and send a note
   from a `send_allow` root.

The marketplace lists only public repositories with the `herdr-plugin`
topic. `skills/knapp/SKILL.md` is the agent skill; it names the read-only
commands, so a new command goes in it too.

## Herdr 0.9.1 reference

| What | Value |
|---|---|
| Env for every command | `HERDR_BIN_PATH`, `HERDR_SOCKET_PATH`, `HERDR_ENV=1`, `HERDR_PLUGIN_ID`, `HERDR_PLUGIN_ROOT`, `HERDR_PLUGIN_CONFIG_DIR`, `HERDR_PLUGIN_STATE_DIR`, `HERDR_PLUGIN_CONTEXT_JSON`, and when known `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`, `HERDR_PANE_ID` |
| Pane commands also get | `HERDR_PLUGIN_ENTRYPOINT_ID`; a popup gets no `HERDR_PANE_ID` |
| Context JSON fields | `workspace_id`, `workspace_label`, `workspace_cwd`, `worktree`, `tab_id`, `tab_label`, `focused_pane_id`, `focused_pane_cwd`, `focused_pane_agent`, `focused_pane_status`, `selected_text`, `invocation_source`, `correlation_id`, `clicked_url`, `link_handler_id` |
| Working directory | the plugin root, for every command |
| Link clicks | Ctrl-click; plain text yields only `http(s)` URLs, OSC 8 hyperlinks yield any URI |
| Clipboard | OSC 52 from a pane is forwarded to the client clipboard, up to 192 KiB |
| CLI output | JSON, `{"id": ..., "result": {...}}`; errors as JSON on stderr, exit 1 |
| Socket | newline-delimited JSON requests and replies |
| Graphics | socket only; `png`, `rgb`, `rgba`, `bgra`; 16 layers per pane; `feature_disabled` when `terminal.kitty_graphics = false` |
| Popup | one at a time; `ui_busy` while another modal is open; `popup.close {}` on the socket closes it |
| Plugin panes | `plugin pane open` without `--target-pane` opens in the focused workspace; a split needs `--target-pane`. `plugin pane focus <id>` / `close <id>`; `pane focus` takes a direction, not an id. A plugin pane's `pane list` entry has `label` = the manifest title and `cwd` = the plugin root |
| Notifications | suppressed for the active tab, though `notification show` answers `shown: true` |
| Links | the socket method `pane.link.activate {pane_id, viewport_row, col}` clicks the link at a cell; the reply says whether a plugin handled it. Handlers are tried in plugin-id order |
| Agent prompt | pastes text unchanged inside `ESC[200~ … ESC[201~` (macOS, Linux) and submits; the argv parser takes the second argument as text even when it starts with `-`, and `--` breaks it |
| Workspace directory | `workspace_cwd` is the focused pane's cwd, which may be another plugin's checkout |
