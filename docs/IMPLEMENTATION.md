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
  file and prints canned JSON from `tests/fixtures/herdr/`. The canned JSON is
  real herdr 0.9.1 output.
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

- Cache file: JSON,
  `{"format": N, "root": "...", "files": {"<rel>": {"kind", "mtime_ns", "size", "parsed"}}}`.
  `const CACHE_FORMAT: u32` lives in `index.rs`, and any change to `Parsed`
  or to what `parse` extracts bumps it.
- Name: SHA-256 (`sha2`) of the canonical root path, hex.
- Load: scan, then compare each entry's mtime and size with the cache.
  Reparse new and changed notes, drop removed ones, then resolve everything.
  Write the cache only if something changed, to a temp file in the same
  directory, then rename.
- Watcher: `notify` 8 `RecommendedWatcher`, recursive on the root, feeding
  the pane's channel. Debounce 150 ms of quiet, then handle the batch:
  - Ignore paths under dot directories or excludes.
  - A content change reparses the file, resolves that note's links, and
    rebuilds `back`.
  - A create, remove, or rename, or an event on a directory, rescans that
    directory. If the set of names changed, resolve everything.
- `index --stats` prints the counts of notes, attachments, links by state,
  and tags, and the times for scan, parse, resolve, and cache write.
  `index --rebuild` deletes the cache first.
- `tests/perf.rs` is `#[ignore]`: it generates 5,000 linked notes in a temp
  dir and prints cold and warm load times. Run it with
  `cargo test --release -- --ignored`.
- Tests: edit, add, rename, and delete files in a temp copy of a fixture,
  reload, and check that states flip. In a copy of `vault-basic`, moving
  `alpha.md` to `x/alpha.md` keeps `[[alpha]]` resolved (one candidate), and
  then adding `y/alpha.md` makes it ambiguous with `x/alpha.md` as the pick.

## Step 3: the pane

- ratatui 0.30, using its `ratatui::crossterm` re-export.
- Threads: one reads crossterm events, one runs the watcher. Both send
  `AppEvent` into one channel, and the main loop redraws after each event.
- `App` state: the index, list mode, list selection, the open note,
  `history: Vec<FileId>` with a cursor, detail scroll, and the selected
  link.
- `render.rs` turns pulldown events into `Vec<Line>` for a given width and
  records a `LinkHit { line, cols, link }` for every link, which `n`, `N`,
  and `enter` use. Cache the result by (note, width).
- The header line shows the root name, the mode, and the `send_allow`
  prefixes, or `send off`.
- Root at start: `KNAPP_CWD`, else `workspace_cwd` from
  `HERDR_PLUGIN_CONTEXT_JSON`, else the current directory, matched against
  the configured roots as the plan says.
- Add the `notes` `[[panes]]` entry to `herdr-plugin.toml`, and open it with
  `herdr plugin pane open --plugin shindakun.knapp --entrypoint notes`.
- Tests: render snapshots from `ratatui::backend::TestBackend` for a fixture
  note at 80 columns, and key sequences that follow a link and go back.

## Step 4: editor, copy, search

- Editor: config `editor`, then `$VISUAL`, then `$EDITOR`, split on
  whitespace (`code -w` works; quoting is not supported). The line argument
  depends on the command's basename: `+LINE FILE` for `vi`, `vim`, `nvim`,
  `nano`, `emacs`, `micro`, `kak`; `FILE:LINE` for `hx`; `-g FILE:LINE` for
  `code`; just `FILE` for anything else. With no editor set, show a status
  line.
- Suspend: disable raw mode, leave the alternate screen, show the cursor,
  run the editor with `Command::status`, then restore and call
  `terminal.clear()` to force a full redraw.
- Copy: write `ESC ] 52 ; c ; <base64> BEL` to stdout. Herdr accepts up to
  192 KiB. The base64 encoder is a few lines of code; no crate.
- `Y`: the shortest trailing part of the note's path, without `.md`, that
  `resolve` from a note at the root sends to this note as `File` (not
  `Ambiguous`). If none does, the full path.
- Search: `rg --json --fixed-strings --smart-case --glob '*.md'` plus
  `--glob '!<exclude>'` for each exclude, run in the root. Read `match`
  records as they arrive. Without `rg`, scan notes with a case-insensitive
  substring match (smart case: case-sensitive when the query has an
  uppercase letter). Search runs in a thread; a new query kills the old
  child.

## Step 5: tags, orphans, recent

- Tags: a tree on `/`. A parent's count includes its children. Each note
  counts once per tag.
- Orphans: notes with an empty `back` entry.
- Recent: notes by mtime, newest first.

## Step 6: send

`send.rs` is pure functions, so it can be tested without herdr:

```rust
pub fn check(root: &Path, allow: &[String], notes: &[PathBuf]) -> Result<(), Refusal>;
pub fn fence(notes: &[(String, String)], suffix: &str) -> String; // (rel path, text)
pub fn suffix() -> String;  // 8 hex chars from 4 bytes of /dev/urandom
```

- `check` canonicalizes the root joined to each prefix and each note. A
  prefix that fails to canonicalize allows nothing. A note passes when some
  canonical prefix equals it or is a whole-component ancestor of it
  (`Path::starts_with`, which compares components). `Refusal` lists every
  refused path.
- `fence` writes the preamble line, then one
  `<knapp-note-SUFFIX path="...">` element per note, with the note text
  unchanged. The caller draws a new suffix while any note text contains
  `knapp-note-SUFFIX`.
- The size check runs on the fenced text.
- Target: `herdr agent list` returns
  `{"result": {"agents": [{"pane_id", "workspace_id", "agent", "agent_status", "focused", "cwd", ...}]}}`.
  Keep the agents whose `workspace_id` is `HERDR_WORKSPACE_ID`. With one,
  send to it. With several, show a picker preselecting the pane id in
  `HERDR_PLUGIN_STATE_DIR/last-agent`. After a send, write that file.
  herdr-llm-lint's `src/herdr.rs` (`parse_agent_list`, `pick_agent`) is
  prior art.
- Send: `herdr agent prompt <pane_id> <text>`. Show herdr's error (such as
  `agent_blocked`) in the status line.
- Tests (`tests/send.rs`), each with a temp dir built at test time: `..` in
  the path, a symlink out of the prefix, `notes-private/` against `notes/`,
  wrong case (skipped when the temp dir's filesystem is case-sensitive, as on
  Linux CI), a prefix that names a file, a
  missing prefix, `S` with one outside backlink, an oversize send, and a
  note containing `</knapp-note-` text. A fake herdr checks that a refusal
  never calls `agent prompt` and that an allowed send passes fenced text.

## Step 7: graph and images

- Subgraph: breadth-first over `forward` and `back` together, up to
  `graph_hops`, capped at 200 nodes as the plan says.
- Tree fallback: the open note, then inbound and outbound neighbours
  indented by hop, marked `<-` and `->`.
- `--dot`: `digraph knapp { "a.md" -> "b.md"; }`, escaping `"` and `\` in
  paths.
- Layout: Fruchterman-Reingold, 300 iterations, with starting positions
  from a hash of each path and the open note pinned at the center. The same
  input always gives the same layout, which tests can rely on.
- Frame: the socket reply to `pane.graphics.info` gives `cell_width_px`,
  `cell_height_px`, `pane_visible`, and `max_layers_per_pane` (16). The
  pixmap is the detail panel's cells times the cell size. Draw edges, then
  nodes (radius from log inbound count), then labels. Rasterize each label
  with `fontdue` and blend it into the `tiny-skia` pixmap. Encode it with
  `Pixmap::encode_png` (tiny-skia's `png-format` feature).
- Socket client in `herdr.rs`: `UnixStream` to `HERDR_SOCKET_PATH`, one JSON
  request per line, one JSON reply per line. Calls:
  - `pane.graphics.info {pane_id}`
  - `pane.graphics.set {pane_id, layer_id, format: "png", image_width, image_height, data_base64, placement: {viewport_col, viewport_row, grid_cols, grid_rows}}`
  - `pane.graphics.clear {pane_id, layer_id}`

  `pane_id` is `HERDR_PANE_ID`. `viewport_col` and `viewport_row` are the
  detail panel's position inside the pane.
- The graph uses layer `graph`. Clear it when leaving the graph, and redraw
  on resize.
- PNG embeds: read width and height from the IHDR chunk (bytes 16 to 23).
  Scale the image to fit the panel width in cells and reserve that many
  blank rendered lines. Place visible images on layers `img-0` to `img-13`,
  clear them on scroll, and place again. Images past the layer budget show
  the placeholder line.
- Font: Noto Sans Regular in `assets/font/` with `OFL.txt`, loaded with
  `include_bytes!`.
- Tests: layout determinism, the node cap, `--dot` output against expected,
  the IHDR reader on the fixture PNG, and the socket calls against a fake
  socket server in the test that records requests.

## Step 8: actions, link handler, peek

- `open-pane`: port herdr-rss `src/launch.rs` (`decide`: open, focus, or
  close from `herdr pane list`), with the pane title `Knapp`. On `Open`, run
  `plugin pane open` with `--env KNAPP_CWD=<workspace_cwd>` and no `--cwd`.
  herdr-rss finds its pane by `label` and `cwd`. Confirm in a real session
  that a knapp pane's `pane list` entry has `label` set to the title before
  relying on it.
- `peek-selection`: read `clicked_url`, else `selected_text`.
  - A `file://` URL may carry a host (`ls --hyperlink` writes
    `file://hostname/path`). Accept an empty host, `localhost`, or this
    machine's hostname; refuse others. Percent-decode the path, and use a
    `#fragment` as the heading to scroll to.
  - Selected text: trim it, strip surrounding `[[ ]]`, quotes, or
    backticks. Try it as an absolute path, then relative to
    `workspace_cwd`, then as a wikilink target in each root in order. The
    first unique match wins.
- `peek`: renders `KNAPP_NOTE`. Keys: scroll, `n`, `N`, `enter` (follows
  inside the popup), `ctrl-o`, `o`, and `q` / `esc` to close. No graphics.
- Manifest: add the `peek` pane, the `open` and `peek-selection` actions, and
  the link handler, exactly as in the plan. The README gets the keybinding
  example.
- Tests: `decide` against captured `pane list` JSON, URL decoding with and
  without a host, selected-text cleanup, and a fake herdr that checks the
  `plugin pane open` argv.

## Step 9: multiple roots

- One `Index` per root, loaded when first switched to. The watcher follows
  the active root only; switching back runs the stat sweep.
- `1`..`9` follow config order. `--root` takes a name or a path.
- Tests: two fixture roots in one config, switching and per-root send
  fences.

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
| Popup | one at a time; `ui_busy` while another modal is open |
