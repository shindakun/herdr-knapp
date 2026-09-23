# Changelog

## 0.1.0 (unreleased)

- `links`, `backlinks`, and `unresolved`: wikilinks, embeds, Markdown
  links, and frontmatter links, resolved the way Obsidian 1.14 resolves
  them, with heading and block fragments checked.
- Config file with named roots and `exclude`. Obsidian's Excluded files are
  honored.
- Parse cache per root, reused when a file's mtime and size match. `index`
  command with `--stats`, `--rebuild`, and `--watch`.
