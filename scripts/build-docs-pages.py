#!/usr/bin/env python3
"""Build a GitHub Pages book from WTFM articles without changing their source."""

from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


repo = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else repo / "docs/book"

with tempfile.TemporaryDirectory(prefix="jiotv-docs-pages-") as staging:
    book = Path(staging)
    source = book / "src"
    shutil.copytree(repo / "docs", source, ignore=shutil.ignore_patterns("book"))

    for article in source.glob("*/index.md"):
        content = article.read_text(encoding="utf-8")
        content, replacements = re.subn(
            r"\A---\r?\n.*?\r?\n---[ \t]*(?:\r?\n|\Z)",
            "",
            content,
            count=1,
            flags=re.S,
        )
        if replacements != 1:
            raise ValueError(f"missing WTFM frontmatter: {article.relative_to(source)}")
        article.write_text(content, encoding="utf-8")

    (book / "book.toml").write_text('[book]\ntitle = "JioTV Help Centre"\nlanguage = "en"\n', encoding="utf-8")
    subprocess.run(["mdbook", "build", "--dest-dir", str(output), str(book)], check=True)
