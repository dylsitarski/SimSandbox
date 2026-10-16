#!/usr/bin/env python3
"""Check the SimSandbox documentation system (see docs/README.md).

Fails (exit 1) when:
  - a Markdown file under docs/ or a root Markdown file is missing from the map in docs/README.md
  - a relative link in any checked Markdown file points to a file that does not exist
  - an ADR in docs/decisions/ is missing from the index in docs/decisions/README.md
  - a living document lacks a "_Last updated: YYYY-MM-DD_" line

Standard library only. Run from anywhere: python3 scripts/check_docs.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
MAP = DOCS / "README.md"
ADR_DIR = DOCS / "decisions"
ADR_INDEX = ADR_DIR / "README.md"
ROOT_DOCS = ["README.md", "AGENTS.md", "CLAUDE.md", "CONTRIBUTING.md", "CHANGELOG.md"]
LIVING = [
    "docs/README.md",
    "docs/STATUS.md",
    "docs/vision.md",
    "docs/roadmap.md",
    "docs/open-questions.md",
    "docs/risks.md",
    "docs/glossary.md",
]

LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
FENCE = re.compile(r"^\s*(```|~~~)")
LAST_UPDATED = re.compile(r"^_Last updated: \d{4}-\d{2}-\d{2}_$", re.MULTILINE)
ADR_NAME = re.compile(r"^\d{4}-.+\.md$")


def links(path: Path) -> list[str]:
    """Relative link targets in a Markdown file, ignoring fenced code blocks."""
    out, in_fence = [], False
    for line in path.read_text(encoding="utf-8").splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for target in LINK.findall(line):
            if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue
            out.append(target.split("#", 1)[0])
    return out


def resolve(source: Path, target: str) -> Path:
    return (source.parent / target).resolve()


def main() -> int:
    errors: list[str] = []
    rel = lambda p: p.relative_to(ROOT).as_posix()  # noqa: E731

    checked = sorted(DOCS.rglob("*.md")) + [ROOT / n for n in ROOT_DOCS if (ROOT / n).exists()]

    # 1. Every document appears in the map.
    mapped = {resolve(MAP, t) for t in links(MAP)}
    for doc in checked:
        if doc != MAP and doc.resolve() not in mapped:
            errors.append(f"{rel(doc)}: not listed in the documentation map (docs/README.md)")

    # 2. Relative links resolve.
    for doc in checked:
        for target in links(doc):
            if target and not resolve(doc, target).exists():
                errors.append(f"{rel(doc)}: broken link -> {target}")

    # 3. Every ADR is in the decision index.
    indexed = {resolve(ADR_INDEX, t) for t in links(ADR_INDEX)}
    for adr in sorted(ADR_DIR.glob("*.md")):
        if ADR_NAME.match(adr.name) and not adr.name.startswith("0000") and adr.resolve() not in indexed:
            errors.append(f"{rel(adr)}: missing from the ADR index (docs/decisions/README.md)")

    # 4. Living documents carry a last-updated date.
    for name in LIVING:
        path = ROOT / name
        if not path.exists():
            errors.append(f"{name}: living document is missing")
        elif not LAST_UPDATED.search(path.read_text(encoding="utf-8")):
            errors.append(f"{name}: missing '_Last updated: YYYY-MM-DD_' line")

    if errors:
        print("Documentation check failed:")
        for e in errors:
            print(f"  - {e}")
        return 1
    print(f"Documentation check passed ({len(checked)} files).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
