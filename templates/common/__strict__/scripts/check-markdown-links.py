#!/usr/bin/env python3

import argparse
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote


INLINE_LINK = re.compile(
    r"!?\[[^\]]*\]\(\s*(<[^>\n]+>|[^\s)]+)(?:\s+[\"'(][^)\n]*[\"')])?\s*\)"
)
REFERENCE_LINK = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*(<[^>\n]+>|\S+)", re.MULTILINE)
EXTERNAL_SCHEME = re.compile(r"^[a-z][a-z\d+.-]*:", re.IGNORECASE)
DEFAULT_ROOTS = ("README.md", "CLAUDE.md", "AGENTS.md", "docs")


def markdown_files(repository_root: Path, roots: list[str]) -> list[Path]:
    history = subprocess.run(
        ["git", "-C", str(repository_root), "rev-parse", "--verify", "HEAD"],
        check=False,
        capture_output=True,
    )
    if history.returncode == 0:
        completed = subprocess.run(
            ["git", "-C", str(repository_root), "ls-files", "-z", "--", *roots],
            check=True,
            capture_output=True,
        )
        return sorted(
            repository_root / Path(raw.decode("utf-8"))
            for raw in completed.stdout.split(b"\0")
            if raw and raw.lower().endswith(b".md")
        )

    markdown: list[Path] = []
    for root in roots:
        path = repository_root / root
        if path.is_file() and path.suffix.lower() == ".md":
            markdown.append(path)
        elif path.is_dir():
            markdown.extend(path.rglob("*.md"))
    return sorted(markdown)


def link_destinations(source: str) -> list[tuple[str, int]]:
    destinations: list[tuple[str, int]] = []
    for pattern in (INLINE_LINK, REFERENCE_LINK):
        destinations.extend((match.group(1), match.start()) for match in pattern.finditer(source))
    return destinations


def local_destination(raw_destination: str) -> str | None:
    destination = raw_destination.strip()
    if destination.startswith("<") and destination.endswith(">"):
        destination = destination[1:-1]
    destination = destination.replace("\\ ", " ")
    if (
        not destination
        or destination.startswith("#")
        or destination.startswith("//")
        or EXTERNAL_SCHEME.match(destination)
    ):
        return None
    path_only = re.split(r"[?#]", destination, maxsplit=1)[0]
    return unquote(path_only) if path_only else None


def missing_links(repository_root: Path, roots: list[str]) -> list[str]:
    missing: list[str] = []
    for markdown_file in markdown_files(repository_root, roots):
        source = markdown_file.read_text(encoding="utf-8")
        for raw_destination, offset in link_destinations(source):
            destination = local_destination(raw_destination)
            if destination is None:
                continue
            if destination.startswith("/"):
                target = repository_root / destination.removeprefix("/")
            else:
                target = markdown_file.parent / destination
            if not target.exists():
                relative_source = markdown_file.relative_to(repository_root)
                line = source.count("\n", 0, offset) + 1
                missing.append(f"{relative_source}:{line} -> {destination}")
    return missing


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check local file links in committed Markdown files."
    )
    parser.add_argument("roots", nargs="*", default=list(DEFAULT_ROOTS))
    args = parser.parse_args()
    repository_root = Path(__file__).resolve().parent.parent
    missing = missing_links(repository_root, args.roots)
    if missing:
        print("Missing local Markdown links:", file=sys.stderr)
        for item in missing:
            print(f"- {item}", file=sys.stderr)
        return 1
    print("All local Markdown links resolve.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
