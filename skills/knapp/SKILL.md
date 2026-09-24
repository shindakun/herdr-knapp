---
name: knapp
description: Query a tree of Markdown notes (an Obsidian vault or a docs folder) by its links from a Herdr session. Use when asked what links to a note, which links are broken, which notes are orphans, what tags are used, or what a note connects to.
---

# knapp

The plugin binary is a read-only CLI over the same link index the notes pane uses. It is `target/release/knapp` under the `plugin_root` that `herdr plugin list` prints. Set the config dir so configured roots resolve by name:

```sh
export HERDR_PLUGIN_CONFIG_DIR="$(herdr plugin config-dir shindakun.knapp)"
```

Every command takes `--root NAME|PATH`: a root name from the config, or a directory. Without it, the root is the configured root containing FILE or the current directory, else the current directory. Links resolve the way Obsidian resolves them. Output is tab-separated.

| Command | Does |
| --- | --- |
| `knapp links FILE` | Each link in FILE: line, state (`resolved`, `ambiguous`, `unresolved`), fragment (`ok`, `missing`, `-`), link as written, target |
| `knapp backlinks FILE` | `source:line` and the linking line for every note that links to FILE |
| `knapp unresolved` | Count, state, and target of every missing or ambiguous link target |
| `knapp unresolved --json` | The same with `sources` (`path`, `line`, `goes_to`) and ambiguous `candidates` |
| `knapp orphans` | Notes no other note links to |
| `knapp tags` | Note count and tag; parents count their children |
| `knapp graph FILE --hops 2` | Notes within N links of FILE as a tree, marked `->`, `<-`, or `<->` |
| `knapp graph FILE --dot` | The same as Graphviz |
| `knapp index --stats` | Note, link, and tag counts for the root |

knapp never writes a note. To fix a broken link, edit the note yourself, then rerun `knapp links FILE` to check it resolves.
