#!/usr/bin/env python3
"""Run the template Markdown link checker against this repository's docs roots."""

import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "templates/common/__strict__/scripts/check-markdown-links.py"
DOCS_ROOTS = (
    "README.md",
    "CLAUDE.md",
    "docs",
    "rust/crates/*/README.md",
    "typescript/packages/*/README.md",
    "cli/README.md",
    "lint/README.md",
    "agent-skills",
    "models",
    "examples",
    "deploy/*/README.md",
)

# Temporary: pages the docs overhaul adds later. Matched by target path; the list only shrinks.
PENDING_TARGET = re.compile(r"(^|/)docs/(guide/[^/]+|adr/\d{4}-[^/]+)\.md$")


def load_checker():
    sys.path.insert(0, str(ROOT / "templates/common/scripts"))
    spec = importlib.util.spec_from_file_location("check_markdown_links", CHECKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def pending(root: Path, item: str) -> bool:
    source, _, destination = item.partition(" -> ")
    target = (root / source.rsplit(":", 1)[0]).parent / destination
    if destination.startswith("/"):
        target = root / destination.removeprefix("/")
    relative = target.resolve().relative_to(root.resolve(), walk_up=True).as_posix()
    return PENDING_TARGET.search(relative) is not None


def check(root: Path, roots: list[str]) -> tuple[int, list[str]]:
    checker = load_checker()
    checked = len(checker.markdown_files(root, roots))
    missing = [item for item in checker.missing_links(root, roots) if not pending(root, item)]
    return checked, missing


def main() -> int:
    roots = sys.argv[1:] or list(DOCS_ROOTS)
    checked, missing = check(ROOT, roots)
    print(f"Checked {checked} Markdown files.")
    if checked == 0:
        print("No Markdown files matched the docs roots.", file=sys.stderr)
        return 1
    if missing:
        print("Missing local Markdown links:", file=sys.stderr)
        for item in missing:
            print(f"- {item}", file=sys.stderr)
        return 1
    print("All local Markdown links resolve.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
