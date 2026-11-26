#!/usr/bin/env python3
"""Run the template Markdown link checker against this repository's docs roots."""

import importlib.util
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


def load_checker():
    sys.path.insert(0, str(ROOT / "templates/common/scripts"))
    spec = importlib.util.spec_from_file_location("check_markdown_links", CHECKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check(root: Path, roots: list[str]) -> tuple[int, list[str]]:
    checker = load_checker()
    return len(checker.markdown_files(root, roots)), checker.missing_links(root, roots)


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
