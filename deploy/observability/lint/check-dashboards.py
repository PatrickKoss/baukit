#!/usr/bin/env python3
"""Check the structure of Grafana dashboard JSON files in pack dashboard directories."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Iterator, Sequence
from pathlib import Path

REQUIRED_KEYS = ("title", "uid", "schemaVersion", "panels", "templating")
BUILTIN_DATASOURCE_UIDS = frozenset({"-- Grafana --", "-- Mixed --", "-- Dashboard --", "__expr__"})
PANELS_WITHOUT_TARGETS = frozenset({"row", "text", "alertlist", "dashlist"})


def variable_name(uid: str) -> str | None:
    if uid.startswith("${") and uid.endswith("}"):
        return uid[2:-1]
    if uid.startswith("$"):
        return uid[1:]
    return None


def datasource_variables(document: dict) -> set[str]:
    entries = document.get("templating", {}).get("list", [])
    return {
        entry["name"]
        for entry in entries
        if isinstance(entry, dict) and entry.get("type") == "datasource" and "name" in entry
    }


def all_panels(panels: list) -> Iterator[dict]:
    for panel in panels:
        if not isinstance(panel, dict):
            continue
        yield panel
        yield from all_panels(panel.get("panels", []))


def datasource_references(value: object, location: str) -> Iterator[tuple[str, object]]:
    if isinstance(value, dict):
        for key, child in value.items():
            child_location = f"{location}.{key}"
            if key == "datasource" and child is not None:
                yield child_location, child
            yield from datasource_references(child, child_location)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from datasource_references(child, f"{location}[{index}]")


def datasource_problem(reference: object, variables: set[str]) -> str | None:
    uid = reference.get("uid") if isinstance(reference, dict) else reference
    if not isinstance(uid, str) or not uid:
        return "datasource has no uid"
    if uid in BUILTIN_DATASOURCE_UIDS:
        return None
    name = variable_name(uid)
    if name is None:
        return f"hard-coded datasource uid {uid!r}; use a datasource variable"
    if name not in variables:
        return f"datasource variable {name!r} is not declared in templating"
    return None


def structure_problems(document: object, location: str) -> list[str]:
    if not isinstance(document, dict):
        return [f"{location}: dashboard must be a JSON object"]
    problems = [
        f"{location}: missing top-level key {key!r}" for key in REQUIRED_KEYS if key not in document
    ]
    if problems:
        return problems
    if not isinstance(document["schemaVersion"], int):
        problems.append(f"{location}: schemaVersion must be an integer")
    templating = document["templating"]
    if not isinstance(templating, dict) or not isinstance(templating.get("list"), list):
        return [*problems, f"{location}: templating.list must be a list"]
    if not isinstance(document["panels"], list):
        return [*problems, f"{location}: panels must be a list"]
    problems.extend(panel_problems(document["panels"], location))
    variables = datasource_variables(document)
    for path, reference in datasource_references(document, location):
        problem = datasource_problem(reference, variables)
        if problem:
            problems.append(f"{path}: {problem}")
    return problems


def panel_problems(panels: list, location: str) -> list[str]:
    problems: list[str] = []
    seen: set[object] = set()
    for panel in all_panels(panels):
        title = panel.get("title", "<untitled>")
        panel_id = panel.get("id")
        if not isinstance(panel_id, int):
            problems.append(f"{location}: panel {title!r} has no integer id")
        elif panel_id in seen:
            problems.append(f"{location}: duplicate panel id {panel_id} ({title!r})")
        seen.add(panel_id)
        if panel.get("type") in PANELS_WITHOUT_TARGETS:
            continue
        if not panel.get("targets"):
            problems.append(f"{location}: panel {title!r} has no targets")
    return problems


def check_directory(directory: Path, uids: dict[str, Path], problems: list[str]) -> int:
    paths = sorted(directory.glob("*.json"))
    for path in paths:
        try:
            document = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            problems.append(f"{path}: invalid dashboard JSON: {error}")
            continue
        problems.extend(structure_problems(document, str(path)))
        uid = document.get("uid") if isinstance(document, dict) else None
        if isinstance(uid, str) and uid in uids:
            problems.append(f"{path}: dashboard uid {uid!r} is also used by {uids[uid]}")
        elif isinstance(uid, str):
            uids[uid] = path
    return len(paths)


def main(argv: Sequence[str] = ()) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directories", nargs="+", type=Path, help="dashboard directories")
    arguments = parser.parse_args(list(argv))
    problems: list[str] = []
    uids: dict[str, Path] = {}
    count = 0
    for directory in arguments.directories:
        if not directory.is_dir():
            print(f"dashboard directory is not a directory: {directory}", file=sys.stderr)
            return 2
        count += check_directory(directory, uids, problems)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print(f"Dashboard structure check passed: {count} dashboard(s).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
