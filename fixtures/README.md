# Fixtures

Each directory is a small note tree the tests index. The expected CLI output
for every note is in `tests/expected/`, written by `scripts/expected.py`;
`docs/IMPLEMENTATION.md` describes the format. After changing a fixture,
update `LINKS` in that script and run `make expected`.

- `vault-basic/`: every wikilink form (alias, heading, nested heading, block,
  `.md` suffix, path), embeds of a note and a PNG, Markdown links with a
  percent-encoded fragment, external links that must be ignored, a missing
  heading, frontmatter of every value shape, and a frontmatter link.
- `vault-ambiguous/`: `a/same.md`, `b/same.md`, and `c/deep/same.md`, linked
  from the root and from `b/`, covering each step of Obsidian's resolution:
  shortest path, same folder first, exact path, path suffix, leading `/`,
  `./` and `../`, and case.
- `vault-broken/`: one target missing three times, another missing once, a
  missing heading, an orphan, a note that links only to itself, and a
  three-note cycle.
- `vault-syntax/`: links and tags inside inline code, fenced code,
  `%%comments%%`, an inline comment, `%%` inside code, an unclosed `%%`, and
  a table with `[[a\|b]]`. Only the links in `tests/expected/` are real.
- `docs-repo/`: a plain docs tree with relative, `./`, `../`,
  percent-encoded, and GitHub-slug fragment links, one that leaves the root,
  and one to a missing file.

Extraction that the CLI output does not show:

| Fixture | Tags | Blocks |
|---|---|---|
| `vault-basic/index.md` | `home`, `meta/index`, `inline-tag`, `nested/child` (not `123`) | none |
| `vault-basic/beta.md` | `heading-tag` | none |
| `vault-basic/alpha.md` | none | `para1` |
| `vault-syntax/syntax.md` | `real-tag` only | none |

`vault-basic/beta.md` frontmatter parses as `title` Str, `draft` Bool,
`created` Date, `aliases` List, `owners` List (`ana`, `[[alpha]]`), and
`weird` Raw.
