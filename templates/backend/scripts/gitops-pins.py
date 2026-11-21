#!/usr/bin/env python3
"""Read and write the `<tag>@sha256:<digest>` image pins in deploy/gitops/<env>/release.yaml."""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

PIN = re.compile(r"[0-9a-f]{12}@sha256:[0-9a-f]{64}")
PIN_LINE = re.compile(
    r'(?m)^(?P<lead>\s+tag: ")(?P<value>[0-9a-f]{12}(?:@sha256:[0-9a-f]{64})?)'
    r'(?P<trail>"\s+# (?P<name>[a-z][a-z0-9-]*) image)$'
)
SOPS_METADATA = re.compile(r"(?m)^sops:\s*$")


def read_pins(path: pathlib.Path) -> dict[str, str]:
    pins: dict[str, str] = {}
    for match in PIN_LINE.finditer(path.read_text(encoding="utf-8")):
        name = match.group("name")
        if name in pins:
            raise SystemExit(f"{path} pins the {name} image twice")
        pins[name] = match.group("value")
    if not pins:
        raise SystemExit(f"{path} holds no image pin lines")
    return pins


def parse_assignments(arguments: list[str]) -> dict[str, str]:
    pins: dict[str, str] = {}
    for argument in arguments:
        name, _, pin = argument.partition("=")
        if not name or name in pins:
            raise SystemExit(f"missing or repeated image name in {argument!r}")
        if not PIN.fullmatch(pin):
            raise SystemExit(f"{name} pin must be <12-hex Git SHA>@sha256:<digest>, got {pin!r}")
        pins[name] = pin
    return pins


def write_pins(path: pathlib.Path, pins: dict[str, str]) -> None:
    found = read_pins(path)
    if sorted(found) != sorted(pins):
        raise SystemExit(f"{path} pins {sorted(found)}, but the release names {sorted(pins)}")
    text = path.read_text(encoding="utf-8")

    def replace(match: re.Match[str]) -> str:
        return f"{match.group('lead')}{pins[match.group('name')]}{match.group('trail')}"

    updated = PIN_LINE.sub(replace, text)
    path.write_text(updated, encoding="utf-8")


def shared_tag(path: pathlib.Path) -> str:
    pins = read_pins(path)
    if not all(PIN.fullmatch(pin) for pin in pins.values()):
        raise SystemExit(f"{path} still holds an unpinned placeholder; release to it first")
    tags = {pin.partition("@")[0] for pin in pins.values()}
    if len(tags) != 1:
        raise SystemExit(f"{path} pins different tags {sorted(tags)}; every image must share one")
    return tags.pop()


def unencrypted_files(directory: pathlib.Path) -> list[pathlib.Path]:
    return sorted(
        path
        for path in directory.rglob("*.enc.yaml")
        if not SOPS_METADATA.search(path.read_text(encoding="utf-8"))
    )


def command_set(arguments: argparse.Namespace) -> None:
    write_pins(arguments.file, parse_assignments(arguments.pins))


def command_copy(arguments: argparse.Namespace) -> None:
    shared_tag(arguments.source)
    write_pins(arguments.target, read_pins(arguments.source))


def command_list(arguments: argparse.Namespace) -> None:
    for name, pin in sorted(read_pins(arguments.file).items()):
        print(f"{name}={pin}")


def command_tag(arguments: argparse.Namespace) -> None:
    print(shared_tag(arguments.file))


def command_check_encrypted(arguments: argparse.Namespace) -> None:
    plain = unencrypted_files(arguments.directory)
    if plain:
        listing = "\n".join(f"  {path}" for path in plain)
        raise SystemExit(f"encrypt these with sops before releasing:\n{listing}")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    set_command = commands.add_parser("set", help="write NAME=TAG@sha256:DIGEST pins")
    set_command.add_argument("file", type=pathlib.Path)
    set_command.add_argument("pins", nargs="+")
    set_command.set_defaults(run=command_set)
    copy_command = commands.add_parser("copy", help="copy every pin from SOURCE to TARGET")
    copy_command.add_argument("source", type=pathlib.Path)
    copy_command.add_argument("target", type=pathlib.Path)
    copy_command.set_defaults(run=command_copy)
    list_command = commands.add_parser("list", help="print NAME=PIN lines")
    list_command.add_argument("file", type=pathlib.Path)
    list_command.set_defaults(run=command_list)
    tag_command = commands.add_parser("tag", help="print the one Git tag every pin shares")
    tag_command.add_argument("file", type=pathlib.Path)
    tag_command.set_defaults(run=command_tag)
    check_command = commands.add_parser("check-encrypted", help="fail on unencrypted *.enc.yaml")
    check_command.add_argument("directory", type=pathlib.Path)
    check_command.set_defaults(run=command_check_encrypted)
    return root


def main(argv: list[str]) -> None:
    arguments = parser().parse_args(argv)
    arguments.run(arguments)


if __name__ == "__main__":
    main(sys.argv[1:])
