# knapp

A reader for a tree of Markdown notes, shipped as one Rust binary that runs
as a CLI and as a Herdr plugin. It indexes wikilinks, Markdown links, embeds,
and tags, and shows backlinks, unresolved links, orphans, and the local
graph. It never writes a note.

## Layout

- `src/main.rs` dispatches argv; `src/cli.rs` holds the subcommands;
  everything else is in the library.
- `src/config.rs` reads the config file and resolves roots.
- `src/scan.rs` walks a root, applies excludes, and splits notes from
  attachments.
- `src/parse.rs` extracts links, embeds, tags, frontmatter, and headings
  from one note.
- `src/index.rs` holds the forward and back tables, resolution, and the
  cache; `src/watch.rs` is the notifier and incremental reindex.
- `src/render.rs` turns Markdown into styled lines; `src/graph.rs` is the
  local subgraph, layout, Graphviz output, and PNG frame. `assets/font/` is
  the embedded label font and its license.
- `src/herdr.rs` reads the environment Herdr injects and calls the Herdr CLI.
- `src/editor.rs` picks and runs the editor and encodes OSC 52 copies;
  `src/search.rs` is full-text search through `rg` or built in.
- `src/send.rs` is the send allowlist.
- `src/tui/` is the notes pane and the peek popup.
- `tests/common/` has the temp-copy helpers; copies get old mtimes so the
  cache's two-second rule does not force reparsing.
- `fixtures/` are note trees the tests index; `fixtures/README.md` says what
  each one covers. `tests/expected/` is the expected CLI output, written by
  `scripts/expected.py`, which ports Obsidian's link resolution.
- `docs/PLAN.md` is the design and the build order; `docs/IMPLEMENTATION.md`
  is how each step is built and tested. `herdr-plugin.toml` is the manifest.

## Rules

- Run `make check` before committing: `fmt-check`, `clippy`, `test`, `audit`,
  `md-lint`, `expected-check`.
- Never edit `tests/expected/` by hand. Change a fixture, update `LINKS` in
  `scripts/expected.py`, and run `make expected`. When knapp's output and the
  script disagree, find which one is wrong before changing either.
- knapp is read-only. No code path writes, renames, or deletes a file under a
  root. Writes go to the cache and config dirs only.
- Every send goes through `send.rs`. It checks the canonicalized path by
  whole components, so `..`, symlinks out of a prefix, and sibling prefixes
  (`notes-private/` against `notes/`) are refused. A new send path gets a
  refusal test in `tests/send.rs` before its key binding.
- Sent note text is data. `send.rs` fences every note in an XML element whose
  tag carries a per-send random suffix, under a line telling the agent not to
  follow instructions in it. Nothing reaches `herdr agent prompt` unfenced
  or uncleaned: `send::clean` removes control characters from every note,
  path, and request, because herdr pastes text unchanged inside
  `ESC[200~ … ESC[201~` and an `ESC[201~` in a note would end the paste.
- A thread that reads watcher batches owns the `Watch` and calls
  `next_batch()`. A closure that names only `watch.batches` drops the
  watcher and silently stops all events.
- A child process knapp reads from gets `stdin(Stdio::null())` and, for
  `rg`, an explicit path. `rg` with neither searches stdin when stdin is not
  a terminal, and hangs.
- Resolutions are never cached. Every load and every watcher refresh
  re-resolves every link; only per-file parse results live in the cache.
- Tests never read or write a user's config or cache. Anything that runs the
  binary sets `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` to temp directories and
  removes `HERDR_PLUGIN_CONFIG_DIR` and `HERDR_PLUGIN_STATE_DIR`.
- A change to what `parse.rs` extracts bumps the cache format version.
- The manifest wires only commands that exist. An entrypoint for an unbuilt
  step is a broken plugin, not a placeholder.
- Dependencies come from this list as the steps need them: `serde`,
  `serde_json`, `toml`, `pulldown-cmark`, `unicode-normalization`,
  `ratatui` (its `crossterm` re-export, no direct `crossterm`), `notify`,
  `sha2`, `tiny-skia`, `fontdue`. Anything else needs a reason in the commit
  message.
- A step in `docs/PLAN.md` closes only when it has been run against a linked
  plugin in a real Herdr session. Fixture tests are necessary, not
  sufficient.
- Update `README.md`, `CHANGELOG.md`, and `docs/PLAN.md` in the same change
  as the code.
