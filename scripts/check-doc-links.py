#!/usr/bin/env python3
"""Check local Markdown links in the Docusaurus documentation source."""
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

root = Path(__file__).resolve().parents[1] / "docs" / "content"
errors = []
link_re = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)|^\s*-\s*\[[^\]]+\]\(([^)]+)\)", re.M)

for source in root.rglob("*.md"):
    text = source.read_text(encoding="utf-8")
    text = re.sub(r"```.*?```|~~~.*?~~~", "", text, flags=re.S)
    for match in link_re.finditer(text):
        raw = (match.group(1) or match.group(2)).strip().split()[0].strip("<>")
        parsed = urlsplit(raw)
        if parsed.scheme or parsed.netloc or not parsed.path:
            continue
        path = (source.parent / unquote(parsed.path)).resolve()
        if path.suffix == "" and path.is_dir():
            path = path / "README.md"
        if not path.is_file():
            errors.append(f"{source.relative_to(root)}: missing {raw}")

for error in errors:
    print(error, file=sys.stderr)
if errors:
    sys.exit(1)
print("All local documentation links point to existing files.")
