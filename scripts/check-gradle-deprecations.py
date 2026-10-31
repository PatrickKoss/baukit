#!/usr/bin/env python3
"""Fail when a Gradle deprecation in a `--warning-mode all` log comes from a Baukit-owned file."""

from __future__ import annotations

import argparse
import re
import sys
import tarfile
from dataclasses import dataclass
from pathlib import Path

DEPRECATION = re.compile(r"has been deprecated|This behavior has been deprecated")
SOURCE_WARNING = re.compile(r"^(?:w: |Note: |C/C\+\+: )|\.(?:java|kt):\d+: ")
SCRIPT_FRAME = re.compile(r"\(([^()]+\.gradle(?:\.kts)?):(\d+)\)")
PLUGIN_FRAME = re.compile(r"^\s*at ((?:com\.android|org\.jetbrains\.kotlin|com\.facebook|expo\.modules)[\w.$]*)")
PROPERTY = re.compile(r"Use assignment \('(\w+) = <value>'\)")
FRAME_WINDOW = 200
TEMPLATE_ARCHIVE = Path("node_modules/expo/template.tgz")


@dataclass(frozen=True)
class Deprecation:
    message: str
    source: str
    owner: str


def first_line(message: str) -> str:
    return message.split(". ")[0]


def frames_after(lines: list[str], index: int) -> list[str]:
    frames = []
    for line in lines[index + 1 : index + 1 + FRAME_WINDOW]:
        if not line.startswith("\tat "):
            if frames:
                break
            continue
        frames.append(line)
    return frames


def locate(message: str, frames: list[str]) -> tuple[str, int | None]:
    prefer_script = PROPERTY.search(message) is not None
    plugin = None
    for frame in frames:
        script = SCRIPT_FRAME.search(frame)
        if script:
            if prefer_script or plugin is None:
                return script.group(1), int(script.group(2))
            break
        match = PLUGIN_FRAME.match(frame)
        if match and plugin is None:
            plugin = match.group(1).split("$")[0]
    return (plugin or "gradle"), None


def template_lines(project: Path, relative: str) -> list[str]:
    archive = project / TEMPLATE_ARCHIVE
    if not archive.is_file():
        return []
    with tarfile.open(archive) as bundle:
        try:
            member = bundle.extractfile(f"package/{relative}")
        except KeyError:
            return []
        if member is None:
            return []
        return [line.strip() for line in member.read().decode().splitlines()]


def from_expo_template(project: Path, path: Path, line: int, message: str) -> bool:
    relative = path.relative_to(project).as_posix()
    template = template_lines(project, relative)
    if not template:
        return False
    generated = path.read_text().splitlines()
    text = generated[line - 1].strip() if 0 < line <= len(generated) else ""
    if text in template:
        return True
    prop = PROPERTY.search(message)
    return prop is not None and any(re.match(rf"{prop.group(1)}\b", entry) for entry in template)


def owner_of(project: Path, source: str, line: int | None, message: str) -> str:
    if line is None:
        return "third-party toolchain"
    path = Path(source).resolve()
    if "node_modules" in path.parts:
        return "third-party package"
    if path.is_relative_to(project / "android") and from_expo_template(project, path, line, message):
        return "expo template"
    return "baukit"


def display(project: Path, source: str, line: int | None) -> str:
    if line is None:
        return source
    text = re.sub(r".*/node_modules/", "node_modules/", source)
    path = Path(text)
    if path.is_absolute() and path.is_relative_to(project):
        text = path.relative_to(project).as_posix()
    return f"{text}:{line}"


def collect(project: Path, log: str) -> list[Deprecation]:
    lines = log.splitlines()
    found: dict[tuple[str, str], Deprecation] = {}
    for index, line in enumerate(lines):
        if not DEPRECATION.search(line) or SOURCE_WARNING.search(line):
            continue
        source, number = locate(line, frames_after(lines, index))
        prop = PROPERTY.search(line)
        message = f"{first_line(line)} ({prop.group(1)})" if prop else first_line(line)
        entry = Deprecation(message, display(project, source, number), owner_of(project, source, number, line))
        found[(entry.message, entry.source)] = entry
    return sorted(found.values(), key=lambda item: (item.owner, item.source, item.message))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", type=Path, required=True, help="Expo project directory")
    parser.add_argument("log", type=Path, help="Gradle output produced with --warning-mode all")
    arguments = parser.parse_args()
    project = arguments.project.resolve()
    deprecations = collect(project, arguments.log.read_text(errors="replace"))
    for item in deprecations:
        print(f"{item.owner}\t{item.source}\t{item.message}")
    owned = [item for item in deprecations if item.owner == "baukit"]
    print(f"{len(deprecations)} Gradle deprecations, {len(owned)} from Baukit-owned files")
    return 1 if owned else 0


if __name__ == "__main__":
    sys.exit(main())
