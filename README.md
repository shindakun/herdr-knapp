# herdr-knapp

A [Herdr](https://herdr.dev) plugin that reads a tree of Markdown notes by
its links. Browse the tree, follow `[[wikilinks]]`, see what links back, find
unresolved links and orphans, and look at the local graph. Works on an
Obsidian vault, a repo's `docs/`, or any directory of `.md` files.

Status: steps 1 to 4 of `docs/PLAN.md` are built: the link index, its
cache and watcher, the CLI, and the notes pane with editing and search. The `open` action and its
keybinding come in a later step; until then, open the pane with:

```sh
herdr plugin pane open --plugin shindakun.knapp --entrypoint notes
```

## The pane

A list on the left (Tree, Backlinks, or Forward; `tab` switches) and the
open note on the right, rendered with its links styled by state. `enter`
opens a note or follows the selected link, `n` / `N` step through links,
`[` / `]` go back and forward, and `?` lists every key. Below 80 columns
the pane shows one side at a time; `h` / `l` switch.

- `o` opens the note in your editor at the selected link's line, and the
  pane updates when you save. The editor is `editor` in the config, else
  `$VISUAL`, else `$EDITOR`, else `vi`. Herdr panes often lack the shell's
  `$EDITOR`, so set `editor` in the config.
- `y` copies the note's path and `Y` a `[[wikilink]]` to it, through the
  terminal's clipboard (OSC 52), which works over SSH under Herdr.
- `/` searches the notes as you type, with `rg` when it is on `PATH`.
  Results are notes the tree shows; `.gitignore` does not hide any.

The pane opens on the workspace's notes: the directory of the workspace's
agent, or a configured root that contains it. It follows changes on disk
while it runs.

## Use it as a CLI

```sh
knapp links FILE [--root NAME|PATH]
knapp backlinks FILE [--root NAME|PATH]
knapp unresolved [--root NAME|PATH] [--json]
knapp index [--root NAME|PATH] [--rebuild] [--stats] [--watch]
```

- `links` prints one line per link in FILE: line, state (`resolved`,
  `ambiguous`, `unresolved`), fragment (`ok`, `missing`, or `-`), the link
  as written, and where it goes. An ambiguous link lists Obsidian's pick
  first, then the other candidates.
- `backlinks` prints `source:line` and the linking line for every note that
  links to FILE.
- `unresolved` counts missing and ambiguous targets across the root.
- `index` loads the root and prints counts. `--stats` adds timings and
  cache use, `--rebuild` ignores the cache, and `--watch` keeps running and
  prints a line for each change to the tree.

Output is tab-separated. Links resolve the way Obsidian does, so a vault's
links point where its author saw them point. Markdown links resolve
relative to their note first, as in a docs repo.

The root is `--root`, a configured root that contains FILE or the current
directory, or else the current directory.

Parsed notes are cached in `~/.cache/knapp/index/` (or
`$XDG_CACHE_HOME/knapp/index/`), one file per root, so later runs only
parse what changed. Deleting the cache is always safe.

## Configure

`~/.config/knapp/config.toml`, or `$XDG_CONFIG_HOME/knapp/config.toml`.
When Herdr runs knapp as a plugin, it reads
`$(herdr plugin config-dir shindakun.knapp)/config.toml` instead:

```toml
exclude = ["node_modules/", "target/"]
editor = "nvim"

[[root]]
name = "notes"
path = "~/notes"
```

Top-level keys go before the first `[[root]]`. Files and folders whose
names start with `.` are always skipped. In an Obsidian vault, the
Excluded files setting applies too.

## Install

```sh
herdr plugin install shindakun/herdr-knapp
```

Needs `cargo`; the install step builds the binary. Linux and macOS.

To work on it, link a checkout instead:

```sh
cargo build --release
herdr plugin link /path/to/herdr-knapp
```

## Development

```sh
make check   # fmt-check, clippy, test, audit, md-lint, expected-check
make hooks   # install the pre-commit hooks
```

`make audit` needs `cargo-audit`; `make md-lint` needs `markdownlint-cli2`;
`make expected` and `make expected-check` need `python3`.
`scripts/release.sh X.Y.Z` cuts a release from a `## X.Y.Z (` section in
`CHANGELOG.md`.

## License

MIT
