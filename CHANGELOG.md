# Changelog

## 0.1.2 (2026-09-23)

- `peek-selection` works after a mouse selection: Herdr drops the
  selection at the key, so knapp reads the clipboard, and uses it when it
  is one line naming a `.md` file or a `[[wikilink]]`.
- The peek popup's footer and `?` help list only the keys it answers to.
- README: plain-text paths are not Ctrl-clickable; the config example uses
  `vim`.

## 0.1.1 (2026-09-23)

- Below 80 columns, opening a note from the list shows it. Before, `enter`
  and clicks opened it behind the list.
- Clicks no longer reach the panes while the help, the agent picker, or
  the send line is open.

## 0.1.0 (2026-09-23)

- `links`, `backlinks`, and `unresolved`: wikilinks, embeds, Markdown
  links, and frontmatter links, resolved the way Obsidian 1.14 resolves
  them, with heading and block fragments checked.
- Config file with named roots and `exclude`. Obsidian's Excluded files are
  honored.
- Notes pane: tree, rendered note, backlinks, and forward links, with link
  following, history, a folded frontmatter table, mouse, and a narrow
  layout. It updates as files change.
- Several roots: `1`..`9` switch between configured roots, each keeping its
  place and staying current.
- `open` action to open, focus, or close the notes pane; Ctrl-click on a
  `file://` link to a note, or `peek-selection` over selected text, opens
  it in a popup at its `#heading`.
- PNG embeds render as images through Herdr's pane graphics.
- `g` shows the local graph, drawn under the text with pane graphics, and
  `knapp graph` prints it as a tree or Graphviz.
- `s` / `S` send the open note, or it and its backlinks, to a workspace
  agent through a send line, fenced as data, with control characters
  removed. `send_allow` sets which paths may be sent.
- Tags, Unresolved, Orphans, and Recent modes, and the `orphans` and
  `tags` commands. `unresolved --json` shows where each ambiguous link
  goes.
- `o` opens the note in the editor and resumes the pane after; `y` / `Y`
  copy the path or a `[[wikilink]]` with OSC 52; `/` searches as you type,
  through `rg` when available.
- Parse cache per root, reused when a file's mtime and size match. `index`
  command with `--stats`, `--rebuild`, and `--watch`.
- An agent skill, `skills/knapp`, for the read-only commands.
