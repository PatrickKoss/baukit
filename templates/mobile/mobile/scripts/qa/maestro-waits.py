"""Print the longest wait a Maestro flow tree declares as `<milliseconds>\\t<command and line>`.

run-maestro.sh widens its stall window to that wait. The scan reads every `timeout:` of a command,
in milliseconds, and follows `runFlow` files relative to the flow that names them.
"""

import os
import re
import sys
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path

USAGE_ERROR = 2
FLOW_SUFFIXES = (".yaml", ".yml")
CONFIG_STEM = "config"
KEY = r"(?P<key>[A-Za-z][\w-]*) *:"
KEY_LINE = re.compile(rf"^(?P<indent> *)(?P<item>- +)?{KEY}(?: +(?P<value>.*))?$")
INLINE_ENTRY = re.compile(r"(?:^|[{,]) *" + KEY + r" *(?P<value>[^,}]*)")
COMMENT = re.compile(r"(?:^| )#.*$")
MILLISECONDS = re.compile(r"\d+")
QUOTES = "'\""


class FlowError(Exception):
    pass


@dataclass(frozen=True)
class Wait:
    milliseconds: int
    where: str


NO_WAIT = Wait(0, "")


@dataclass(frozen=True)
class Entry:
    key: str
    value: str
    command: str


@dataclass
class FlowScan:
    longest: Wait = NO_WAIT
    references: list[Path] = field(default_factory=list)


def unquote(value: str) -> str:
    value = value.strip()
    if len(value) > 1 and value[0] == value[-1] and value[0] in QUOTES:
        return value[1:-1]
    return value


def display(path: Path) -> str:
    return os.path.normpath(path)


def read_lines(flow: Path) -> list[str]:
    try:
        return flow.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        raise FlowError(f"cannot read Maestro flow {display(flow)}: {error}") from error


def line_entries(match: re.Match[str], parent: str) -> list[Entry]:
    key, value = match["key"], unquote(match["value"] or "")
    if not value.startswith("{"):
        return [Entry(key, value, parent)]
    inline = [
        Entry(entry["key"], unquote(entry["value"]), key)
        for entry in INLINE_ENTRY.finditer(value[1:])
    ]
    return [Entry(key, "", parent), *inline]


def wait_of(entry: Entry, where: str) -> Wait:
    if not MILLISECONDS.fullmatch(entry.value):
        raise FlowError(
            f"{where}: {entry.command or 'the'} timeout must be whole milliseconds,"
            f" got {entry.value!r}"
        )
    return Wait(int(entry.value), f"{entry.command or 'a command'} in {where}")


def reference_of(entry: Entry, flow: Path, where: str) -> Path | None:
    names_flow = (entry.key == "runFlow" and entry.value) or (
        entry.key == "file" and entry.command == "runFlow"
    )
    if not names_flow:
        return None
    if "${" in entry.value:
        raise FlowError(f"{where}: cannot follow runFlow {entry.value}, it names a variable")
    reference = flow.parent / entry.value
    if not reference.is_file():
        raise FlowError(f"{where}: runFlow file {display(reference)} does not exist")
    return reference


def scan(flow: Path) -> FlowScan:
    result = FlowScan()
    commands: list[tuple[int, str]] = []
    for number, raw in enumerate(read_lines(flow), start=1):
        match = KEY_LINE.match(COMMENT.sub("", raw).rstrip())
        if match is None:
            continue
        column = len(match["indent"]) + len(match["item"] or "")
        while commands and commands[-1][0] >= column:
            commands.pop()
        parent = commands[-1][1] if commands else ""
        if match["item"]:
            commands.append((column, match["key"]))
        where = f"{display(flow)}:{number}"
        for entry in line_entries(match, parent):
            if entry.key == "timeout":
                wait = wait_of(entry, where)
                if wait.milliseconds > result.longest.milliseconds:
                    result.longest = wait
            reference = reference_of(entry, flow, where)
            if reference is not None:
                result.references.append(reference)
    return result


def flow_files(argument: Path) -> list[Path]:
    if argument.is_file():
        return [argument]
    if not argument.is_dir():
        raise FlowError(f"Maestro flow {display(argument)} does not exist")
    return sorted(
        path
        for path in argument.rglob("*")
        if path.suffix in FLOW_SUFFIXES and path.stem != CONFIG_STEM and path.is_file()
    )


def longest_wait(arguments: list[Path]) -> Wait:
    pending = deque(flow for argument in arguments for flow in flow_files(argument))
    seen: set[Path] = set()
    longest = NO_WAIT
    while pending:
        flow = pending.popleft()
        if flow.resolve() in seen:
            continue
        seen.add(flow.resolve())
        result = scan(flow)
        if result.longest.milliseconds > longest.milliseconds:
            longest = result.longest
        pending.extend(result.references)
    return longest


def main(arguments: list[str]) -> int:
    try:
        wait = longest_wait([Path(argument) for argument in arguments])
    except FlowError as error:
        print(f"qa: {error}", file=sys.stderr)
        return USAGE_ERROR
    print(f"{wait.milliseconds}\t{wait.where}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
