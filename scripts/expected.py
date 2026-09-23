#!/usr/bin/env python3
"""Regenerate tests/expected/ from the fixtures.

    python3 scripts/expected.py            # rewrite tests/expected/
    python3 scripts/expected.py --check    # exit 1 if tests/expected/ is stale

Resolution is a port of Obsidian 1.14.2's getLinkpathDest and
resolveSubpath, plus knapp's additions from docs/PLAN.md: NFC keys, equal
lengths ordered by path, and Markdown links resolved relative to the source
note first. Extraction is not ported. LINKS lists each note's links by hand,
in file order, written exactly as in the source; a link missing from the
note is an error. When a fixture changes, update LINKS and run this.
"""
import filecmp
import os
import posixpath
import re
import sys
import tempfile
import unicodedata
from urllib.parse import unquote

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURES = os.path.join(REPO, "fixtures")
EXPECTED = os.path.join(REPO, "tests", "expected")

LINKS = {
    "vault-basic": {
        "index.md": [
            "[[dir/deep]]",
            "[[alpha]]",
            "[[Alpha|the alpha note]]",
            "[[alpha#Second Section]]",
            "[[alpha#^para1]]",
            "[[alpha.md]]",
            "[[dir/deep]]",
            "![[img.png]]",
            "![[beta]]",
            "[[beta#Top#Inner]]",
            "[[beta#Nowhere]]",
            "[beta](beta.md)",
            "[deep](dir/deep.md#Deep%20Notes)",
        ],
        "alpha.md": ["[[index]]"],
        "beta.md": ["[[alpha]]"],
        "dir/deep.md": ["[index](../index.md)"],
    },
    "vault-ambiguous": {
        "root.md": ["[[same]]", "[[b/same]]", "[[deep/same]]", "[[/a/same]]", "[[SAME]]"],
        "b/linker.md": ["[[same]]", "[[./same]]", "[[../a/same]]"],
    },
    "vault-broken": {
        "hub.md": [
            "[[missing]]",
            "[[missing]]",
            "[[Other Missing]]",
            "[[ref]]",
            "[[loop-a#No Such Heading]]",
        ],
        "ref.md": ["[[missing]]", "[[loop-a]]"],
        "orphan.md": ["[[hub]]"],
        "self.md": ["[[self]]", "[[#Self]]"],
        "loop-a.md": ["[[loop-b]]"],
        "loop-b.md": ["[[loop-c]]"],
        "loop-c.md": ["[[loop-a]]"],
    },
    "vault-syntax": {
        "syntax.md": [
            "[[real]]",
            "[[after-comment]]",
            "[[table-target\\|shown]]",
            "[[after-code-percent]]",
        ],
    },
    "docs-repo": {
        "README.md": [
            "[Guide](docs/guide.md)",
            "[Setup](docs/guide.md#getting-started)",
            "[Spaces](docs/my%20notes.md)",
            "[Outside](../outside.md)",
            "[Missing](docs/missing.md)",
        ],
        "docs/guide.md": ["[readme](../README.md)", "[API](./api/ref.md)"],
        "docs/api/ref.md": ["[guide](../guide.md#guide)"],
    },
}

# Each note's tags, by hand, in file order (frontmatter first). Notes not
# listed have none.
TAGS = {
    "vault-basic": {
        "index.md": ["home", "meta/index", "inline-tag", "nested/child"],
        "beta.md": ["heading-tag"],
    },
    "vault-syntax": {"syntax.md": ["real-tag"]},
}

HEADING_PUNCT = re.compile(r'[!"#$%&()*+,.:;<=>?@^`{|}~/\[\]\\\r\n]')


def key(s):
    return unicodedata.normalize("NFC", s).lower()


def basename(path):
    return path.rsplit("/", 1)[-1]


def dirname(path):
    return path.rsplit("/", 1)[0] if "/" in path else ""


class Vault:
    def __init__(self, root):
        self.root = root
        self.files = []
        for folder, dirs, names in os.walk(root):
            dirs[:] = [d for d in dirs if not d.startswith(".")]
            for name in names:
                if not name.startswith("."):
                    rel = os.path.relpath(os.path.join(folder, name), root)
                    self.files.append(rel.replace(os.sep, "/"))
        self.files.sort()
        self.by_name = {}
        for path in self.files:
            self.by_name.setdefault(key(basename(path)), []).append(path)

    def wikilink(self, target, source):
        """Obsidian's getLinkpathDest: candidates, pick first."""
        if target == "":
            return [source]
        link = key(target)
        name = basename(link)
        candidates = self.by_name.get(name) if "." in name else None
        if not candidates:
            link = key(target + ".md")
            name = basename(link)
            candidates = self.by_name.get(name)
        if not candidates:
            return []
        if name == link and len(candidates) == 1:
            return list(candidates)
        folder = key(dirname(source))
        if link.startswith("./") or link.startswith("../"):
            if link.startswith("./../"):
                link = link[2:]
            if link.startswith("./"):
                link = (folder + "/" if folder else "") + link[2:]
            else:
                while link.startswith("../"):
                    link = link[3:]
                    folder = dirname(folder)
                link = (folder + "/" if folder else "") + link
            for path in candidates:
                if key(path) == link:
                    return [path]
        if link.startswith("/"):
            link = link[1:]
        for path in candidates:
            if key(path) == link:
                return [path]
        if target.startswith("/"):
            return []
        # Obsidian's plain string tests, kept on purpose.
        kept = [p for p in candidates if key(p).endswith(link)]
        near = sorted((p for p in kept if key(p).startswith(folder)), key=lambda p: (len(p), p))
        far = sorted((p for p in kept if not key(p).startswith(folder)), key=lambda p: (len(p), p))
        return near + far

    def markdown(self, target, source):
        joined = posixpath.normpath(posixpath.join(dirname(source), target))
        if joined == ".." or joined.startswith("../"):
            return []
        for path in self.files:
            if key(path) == key(joined):
                return [path]
        return self.wikilink(target, source)


def heading_norm(s):
    return re.sub(r"\s+", " ", HEADING_PUNCT.sub(" ", s)).strip().lower()


def github_slug(s):
    return re.sub(r"[^\w\- ]", "", s.lower()).replace(" ", "-")


def headings(text):
    found, fenced = [], False
    for line in text.split("\n"):
        if line.startswith("```"):
            fenced = not fenced
            continue
        m = None if fenced else re.match(r"(#{1,6}) (.*)", line)
        if m:
            found.append((len(m.group(1)), m.group(2).strip()))
    return found


def block_ids(text):
    return {m.lower() for m in re.findall(r" \^([A-Za-z0-9-]+)$", text, re.M)}


def fragment_found(root, path, fragment, is_markdown):
    """Obsidian's resolveSubpath, plus GitHub slugs for Markdown links."""
    with open(os.path.join(root, path)) as f:
        text = f.read()
    parts = [p for p in fragment.split("#") if p]
    if len(parts) == 1 and parts[0].startswith("^"):
        return parts[0][1:].lower() in block_ids(text)
    matched, depth = 0, 0
    for level, heading in headings(text):
        part = parts[matched]
        if level > depth and (
            heading_norm(heading) == heading_norm(part)
            or (is_markdown and github_slug(heading) == part.lower())
        ):
            matched, depth = matched + 1, level
            if matched == len(parts):
                return True
    return False


def split_link(written):
    """(is_markdown, target, fragment or None) for one link as written."""
    if written.startswith("[[") or written.startswith("![["):
        body = written.lstrip("!")[2:-2].split("|")[0]
        body = body.removesuffix("\\")
        is_markdown = False
    else:
        body = unquote(re.match(r"\[.*?\]\((.*)\)", written).group(1))
        is_markdown = True
    target, hash_, fragment = body.partition("#")
    return is_markdown, target, fragment if hash_ else None


def tag_lines(tags_by_note):
    """COUNT<TAB>TAG lines: tags compare without case and show their most used
    spelling (ties to the first seen); parents count their children's notes."""
    spellings = {}  # key -> {spelling: [occurrences, first seen]}
    notes = {}  # key -> notes tagged exactly
    order = 0
    for note in sorted(tags_by_note):
        for tag in tags_by_note[note]:
            prefix = tag.rstrip("/")
            while True:
                s = spellings.setdefault(prefix.lower(), {}).setdefault(prefix, [0, order])
                s[0] += 1
                order += 1
                notes.setdefault(prefix.lower(), set())
                if prefix == tag.rstrip("/"):
                    notes[prefix.lower()].add(note)
                if "/" not in prefix:
                    break
                prefix = prefix.rsplit("/", 1)[0]

    def subtree(k):
        out = set(notes[k])
        for other in notes:
            if other.startswith(k + "/"):
                out |= notes[other]
        return out

    def walk(parent):
        kids = sorted(k for k in notes if (k.rsplit("/", 1)[0] if "/" in k else None) == parent)
        for k in kids:
            shown = max(spellings[k].items(), key=lambda s: (s[1][0], -s[1][1]))[0]
            yield f"{len(subtree(k))}\t{shown}\n"
            yield from walk(k)

    return "".join(walk(None))


def file_part(path):
    return path.replace("/", "_")


def generate(out):
    for fixture, notes in LINKS.items():
        root = os.path.join(FIXTURES, fixture)
        vault = Vault(root)
        records = []
        for note, links in notes.items():
            with open(os.path.join(root, note)) as f:
                text = f.read()
            lines = text.split("\n")
            pos, rows = 0, []
            for written in links:
                start = text.index(written, pos)
                pos = start + len(written)
                line = text.count("\n", 0, start) + 1
                is_markdown, target, fragment = split_link(written)
                resolve = vault.markdown if is_markdown else vault.wikilink
                found = resolve(target, note)
                state = "unresolved" if not found else "resolved" if len(found) == 1 else "ambiguous"
                if fragment is None or not found:
                    frag = "-"
                elif fragment_found(root, found[0], fragment, is_markdown):
                    frag = "ok"
                else:
                    frag = "missing"
                rows.append((line, state, frag, written, ",".join(found) or "-"))
                records.append(
                    {"source": note, "line": line, "state": state, "target": target,
                     "found": found, "text": lines[line - 1].strip()}
                )
            with open(os.path.join(out, f"{fixture}.links.{file_part(note)}.txt"), "w") as f:
                for row in rows:
                    f.write("\t".join(map(str, row)) + "\n")

        backlinks = {}
        for r in records:
            if r["found"] and r["found"][0] != r["source"]:
                backlinks.setdefault(r["found"][0], set()).add((r["source"], r["line"], r["text"]))
        for target, hits in backlinks.items():
            with open(os.path.join(out, f"{fixture}.backlinks.{file_part(target)}.txt"), "w") as f:
                for source, line, text in sorted(hits):
                    f.write(f"{source}:{line}\t{text}\n")

        with open(os.path.join(out, f"{fixture}.tags.txt"), "w") as f:
            f.write(tag_lines(TAGS.get(fixture, {})))
        notes_here = [p for p in vault.files if p.lower().endswith(".md")]
        with open(os.path.join(out, f"{fixture}.orphans.txt"), "w") as f:
            for note in notes_here:
                if note not in backlinks:
                    f.write(note + "\n")

        groups = {}
        # sorted() is stable: path order, then each note's link order.
        for r in sorted(records, key=lambda r: r["source"]):
            if r["state"] != "resolved":
                group = (r["state"], key(r["target"]).removesuffix(".md"))
                groups.setdefault(group, [0, r["target"]])[0] += 1
        order = sorted(groups.items(), key=lambda g: (-g[1][0], g[0][0] != "unresolved", g[0][1]))
        with open(os.path.join(out, f"{fixture}.unresolved.txt"), "w") as f:
            for (state, _), (count, target) in order:
                f.write(f"{count}\t{state}\t{target}\n")


def main():
    if sys.argv[1:] == ["--check"]:
        with tempfile.TemporaryDirectory() as tmp:
            generate(tmp)
            diff = filecmp.dircmp(tmp, EXPECTED)
            stale = diff.left_only + diff.right_only + diff.diff_files
            stale += filecmp.cmpfiles(tmp, EXPECTED, diff.same_files, shallow=False)[1]
        if stale:
            print("tests/expected/ is stale; run python3 scripts/expected.py:", *sorted(set(stale)), sep="\n  ")
            sys.exit(1)
        return
    if sys.argv[1:]:
        sys.exit(__doc__)
    os.makedirs(EXPECTED, exist_ok=True)
    for name in os.listdir(EXPECTED):
        if name.endswith(".txt"):
            os.remove(os.path.join(EXPECTED, name))
    generate(EXPECTED)


if __name__ == "__main__":
    main()
