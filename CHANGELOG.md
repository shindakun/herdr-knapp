# Changelog

## 0.1.0 (unreleased)

- `links`, `backlinks`, and `unresolved`: wikilinks, embeds, Markdown
  links, and frontmatter links, resolved the way Obsidian 1.14 resolves
  them, with heading and block fragments checked.
- Config file with named roots and `exclude`. Obsidian's Excluded files are
  honored.
- Notes pane: tree, rendered note, backlinks, and forward links, with link
  following, history, a folded frontmatter table, mouse, and a narrow
  layout. It updates as files change.
- Tags, Unresolved, Orphans, and Recent modes, and the `orphans` and
  `tags` commands. `unresolved --json` shows where each ambiguous link
  goes.
- `o` opens the note in the editor and resumes the pane after; `y` / `Y`
  copy the path or a `[[wikilink]]` with OSC 52; `/` searches as you type,
  through `rg` when available.
- Parse cache per root, reused when a file's mtime and size match. `index`
  command with `--stats`, `--rebuild`, and `--watch`.
