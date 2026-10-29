#!/usr/bin/env python3
"""Check WTFM article metadata, local links and screenshot references."""
from pathlib import Path
import re
import sys
from collections import Counter
from urllib.parse import unquote, urlsplit

root = Path(__file__).resolve().parents[1] / "docs"
errors = []
link_re = re.compile(r"(!?)\[([^\]]*)\]\(([^)]+)\)")

for source in root.rglob("*.md"):
    text = source.read_text(encoding="utf-8")
    if source.name not in {"README.md", "SUMMARY.md"}:
        if source.name != "index.md" or source.parent == root:
            errors.append(f"{source.relative_to(root)}: expected WTFM layout docs/<slug>/index.md")
        frontmatter = re.match(r"\A---\r?\n(.*?)\r?\n---[ \t]*(?:\r?\n|\Z)", text, re.S)
        if not frontmatter:
            errors.append(f"{source.relative_to(root)}: missing frontmatter")
        else:
            fields = frontmatter.group(1)
            for key in ("title", "description", "category", "keywords"):
                if not re.search(rf"^{key}:[ \t]*\S[^\r\n]*$", fields, re.M):
                    errors.append(f"{source.relative_to(root)}: missing {key} metadata")
    text = re.sub(r"```.*?```|~~~.*?~~~", "", text, flags=re.S)
    for match in link_re.finditer(text):
        is_image = bool(match.group(1))
        if is_image and not match.group(2).strip():
            errors.append(f"{source.relative_to(root)}: image missing alt text")
        raw = match.group(3).strip().split()[0].strip("<>")
        parsed = urlsplit(raw)
        if parsed.scheme or parsed.netloc or not parsed.path:
            continue
        path = (source.parent / unquote(parsed.path)).resolve()
        if path.suffix == "" and path.is_dir():
            path = path / "README.md"
        if not path.is_file():
            errors.append(f"{source.relative_to(root)}: missing {'image' if is_image else 'link'} {raw}")

# WTFM discovers articles by docs/<slug>/index.md. Every one must also be
# reachable from the hand-maintained reader navigation page.
summary = root / "SUMMARY.md"
nav_targets = []
for match in link_re.finditer(summary.read_text(encoding="utf-8")):
    raw = match.group(3).strip().split()[0].strip("<>")
    parsed = urlsplit(raw)
    if parsed.scheme or parsed.netloc:
        continue
    path = (summary.parent / unquote(parsed.path)).resolve()
    if path.name == "index.md":
        nav_targets.append(path)

nav_counts = Counter(nav_targets)
for article in root.glob("*/index.md"):
    if nav_counts[article.resolve()] == 0:
        errors.append(f"{article.relative_to(root)}: missing from SUMMARY.md navigation")
    elif nav_counts[article.resolve()] > 1:
        errors.append(f"{article.relative_to(root)}: repeated in SUMMARY.md navigation")

for error in errors:
    print(error, file=sys.stderr)
if errors:
    sys.exit(1)
print("WTFM article metadata, local links and images validated.")
