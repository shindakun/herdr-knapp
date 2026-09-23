# herdr-knapp

A [Herdr](https://herdr.dev) plugin that reads a tree of Markdown notes by
its links. Browse the tree, follow `[[wikilinks]]`, see what links back, find
unresolved links and orphans, and look at the local graph. Works on an
Obsidian vault, a repo's `docs/`, or any directory of `.md` files.

Status: step 1 of `docs/PLAN.md` is built: the link index and three CLI
commands. The Herdr pane is not built yet, so the manifest has no entry
points.

## Use it as a CLI

```sh
knapp links FILE [--root NAME|PATH]
knapp backlinks FILE [--root NAME|PATH]
knapp unresolved [--root NAME|PATH] [--json]
```

- `links` prints one line per link in FILE: line, state (`resolved`,
  `ambiguous`, `unresolved`), fragment (`ok`, `missing`, or `-`), the link
  as written, and where it goes. An ambiguous link lists Obsidian's pick
  first, then the other candidates.
- `backlinks` prints `source:line` and the linking line for every note that
  links to FILE.
- `unresolved` counts missing and ambiguous targets across the root.

Output is tab-separated. Links resolve the way Obsidian does, so a vault's
links point where its author saw them point. Markdown links resolve
relative to their note first, as in a docs repo.

The root is `--root`, a configured root that contains FILE or the current
directory, or else the current directory.

## Configure

`~/.config/knapp/config.toml`, or `$XDG_CONFIG_HOME/knapp/config.toml`.
When Herdr runs knapp as a plugin, it reads
`$(herdr plugin config-dir shindakun.knapp)/config.toml` instead:

```toml
exclude = ["node_modules/", "target/"]

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
