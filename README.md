# herdr-knapp

A [Herdr](https://herdr.dev) plugin that reads a tree of Markdown notes by
its links. Browse the tree, follow `[[wikilinks]]`, see what links back, find
unresolved links and orphans, and look at the local graph. Works on an
Obsidian vault, a repo's `docs/`, or any directory of `.md` files.

Status: not built. `docs/PLAN.md` is the design and the build order; the
binary answers `help` and `version`.

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
