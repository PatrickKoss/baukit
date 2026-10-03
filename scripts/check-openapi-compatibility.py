#!/usr/bin/env python3
"""Report OpenAPI changes that can break a client written against a base document.

A change is breaking when a client that conformed to the base document could fail or lose data
against the current one. When the script cannot decide, it reports the change; a product records
an intentional break in an accepted-breaks file with a reason.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path


METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
PATH_PARAMETER = re.compile(r"\{[^}/]+\}")
SUCCESS_STATUS = re.compile(r"^2(\d\d|XX)$")
REQUEST = "request"
RESPONSE = "response"
LOWER_BOUNDS = ("minimum", "exclusiveMinimum", "minLength", "minItems", "minProperties")
UPPER_BOUNDS = ("maximum", "exclusiveMaximum", "maxLength", "maxItems", "maxProperties")
ALTERNATIVES = ("oneOf", "anyOf")

RULES = {
    "operation-removed": "an operation was removed",
    "operation-id-changed": "an operationId changed",
    "security-changed": "a credential the base accepted is no longer enough",
    "parameter-removed": "a parameter was removed",
    "parameter-required": "a parameter became required or was added as required",
    "request-body-required": "a request body became required",
    "request-media-type-removed": "a request media type was removed",
    "response-removed": "a documented response status was removed",
    "success-status-added": "a new success status was added",
    "response-media-type-removed": "a response media type was removed",
    "type-changed": "a schema accepts or returns different types",
    "enum-narrowed": "a request enum lost values",
    "enum-widened": "a response enum gained values",
    "property-removed": "an object property was removed",
    "required-added": "a request property became required",
    "required-removed": "a response property is no longer required",
    "additional-properties-closed": "a request object no longer accepts extra properties",
    "constraint-narrowed": "a request constraint became stricter",
    "constraint-widened": "a response constraint became looser",
    "pattern-changed": "a string pattern changed",
    "format-changed": "a string format changed",
    "default-changed": "a request default changed",
    "composition-changed": "oneOf, anyOf, or allOf changed incompatibly",
}


class ConfigurationError(Exception):
    """Invalid input that prevents the comparison."""


@dataclass(frozen=True)
class Break:
    rule: str
    location: str
    message: str

    def render(self) -> str:
        return f"{self.rule} {self.location}: {self.message}"


@dataclass(frozen=True)
class Accepted:
    rule: str
    location: str
    reason: str


class Comparison:
    """Compares a base document with a current one and collects breaking changes."""

    def __init__(self, base: dict, current: dict, prefixes: Sequence[str]) -> None:
        self.base = base
        self.current = current
        self.prefixes = tuple(prefixes)
        self.breaks: list[Break] = []

    def report(self, rule: str, location: str, message: str) -> None:
        self.breaks.append(Break(rule, location, message))

    def run(self) -> list[Break]:
        base_operations = operations(self.base, self.prefixes)
        current_operations = operations(self.current, self.prefixes)
        for key, (label, base_operation, base_item) in base_operations.items():
            if key not in current_operations:
                self.report("operation-removed", label, "the operation no longer exists")
                continue
            current_label, current_operation, current_item = current_operations[key]
            self.compare_operation(
                current_label,
                (base_operation, base_item),
                (current_operation, current_item),
            )
        return self.breaks

    def compare_operation(self, label: str, base: tuple, current: tuple) -> None:
        base_operation, (base_path, base_item) = base
        current_operation, (current_path, current_item) = current
        base_id = base_operation.get("operationId")
        current_id = current_operation.get("operationId")
        if base_id is not None and base_id != current_id:
            self.report(
                "operation-id-changed", label, f"operationId {base_id!r} became {current_id!r}"
            )
        self.compare_security(label, base_operation, current_operation)
        self.compare_parameters(
            label,
            parameters(self.base, base_path, base_item, base_operation),
            parameters(self.current, current_path, current_item, current_operation),
        )
        self.compare_request_body(label, base_operation, current_operation)
        self.compare_responses(label, base_operation, current_operation)

    def compare_security(self, label: str, base_operation: dict, current_operation: dict) -> None:
        base_options = security_options(self.base, base_operation)
        current_options = security_options(self.current, current_operation)
        for option in base_options:
            if not any(satisfies(option, candidate) for candidate in current_options):
                shown = describe_security(option)
                self.report(
                    "security-changed",
                    f"{label} security",
                    f"a client that sends {shown} is no longer accepted",
                )

    def compare_parameters(self, label: str, base: dict, current: dict) -> None:
        for key, parameter in current.items():
            if not parameter.get("required"):
                continue
            base_parameter = base.get(key)
            if base_parameter is None or not base_parameter.get("required"):
                self.report(
                    "parameter-required",
                    f"{label} parameter {describe_parameter(key, parameter)}",
                    "clients that omit it are rejected",
                )
        for key, base_parameter in base.items():
            location = f"{label} parameter {describe_parameter(key, base_parameter)}"
            parameter = current.get(key)
            if parameter is None:
                self.report("parameter-removed", location, "the parameter no longer exists")
                continue
            self.compare_schema(
                parameter_schema(base_parameter),
                parameter_schema(parameter),
                REQUEST,
                location,
            )

    def compare_request_body(
        self, label: str, base_operation: dict, current_operation: dict
    ) -> None:
        base_body = resolve(self.base, base_operation.get("requestBody"))
        current_body = resolve(self.current, current_operation.get("requestBody"))
        location = f"{label} request"
        if current_body is not None and current_body.get("required"):
            if base_body is None or not base_body.get("required"):
                self.report(
                    "request-body-required", location, "clients that omit the body are rejected"
                )
        if base_body is None or current_body is None:
            return
        self.compare_content(
            location,
            base_body.get("content", {}),
            current_body.get("content", {}),
            REQUEST,
            "request-media-type-removed",
        )

    def compare_responses(self, label: str, base_operation: dict, current_operation: dict) -> None:
        base_responses = base_operation.get("responses", {})
        current_responses = current_operation.get("responses", {})
        for status in current_responses:
            if status not in base_responses and SUCCESS_STATUS.match(status):
                self.report(
                    "success-status-added",
                    f"{label} response {status}",
                    "clients written against the base do not expect this success status",
                )
        for status, base_response in base_responses.items():
            location = f"{label} response {status}"
            if status not in current_responses:
                self.report("response-removed", location, "the status is no longer documented")
                continue
            self.compare_content(
                location,
                resolve(self.base, base_response).get("content", {}),
                resolve(self.current, current_responses[status]).get("content", {}),
                RESPONSE,
                "response-media-type-removed",
            )

    def compare_content(
        self, location: str, base: dict, current: dict, direction: str, removed_rule: str
    ) -> None:
        for media_type, base_media in base.items():
            media_location = f"{location} {media_type}"
            if media_type not in current:
                self.report(removed_rule, media_location, "the media type is no longer documented")
                continue
            self.compare_schema(
                base_media.get("schema"),
                current[media_type].get("schema"),
                direction,
                media_location,
            )

    def compare_schema(
        self, base: object, current: object, direction: str, location: str
    ) -> None:
        SchemaComparison(self, direction, location).compare(base, current, "$", frozenset())


class SchemaComparison:
    """Compares two schemas in one direction: what a client sends or what it receives."""

    def __init__(self, comparison: Comparison, direction: str, location: str) -> None:
        self.comparison = comparison
        self.direction = direction
        self.location = location

    @property
    def is_request(self) -> bool:
        return self.direction == REQUEST

    def report(self, rule: str, pointer: str, message: str) -> None:
        self.comparison.report(rule, f"{self.location} {pointer}", message)

    def compare(self, base: object, current: object, pointer: str, seen: frozenset) -> None:
        pair = (reference(base), reference(current))
        if pair != (None, None):
            if pair in seen:
                return
            seen = seen | {pair}
        base = resolve(self.comparison.base, base)
        current = resolve(self.comparison.current, current)
        if not isinstance(base, dict) or not isinstance(current, dict):
            return
        self.compare_types(base, current, pointer)
        self.compare_enum(base, current, pointer)
        self.compare_bounds(base, current, pointer)
        self.compare_strings(base, current, pointer)
        self.compare_default(base, current, pointer)
        self.compare_object(base, current, pointer, seen)
        self.compare_items(base, current, pointer, seen)
        self.compare_composition(base, current, pointer, seen)

    def compare_types(self, base: dict, current: dict, pointer: str) -> None:
        base_types = schema_types(base)
        current_types = schema_types(current)
        if self.is_request:
            narrower = not covers(current_types, base_types)
        else:
            narrower = not covers(base_types, current_types)
        if narrower:
            self.report(
                "type-changed",
                pointer,
                f"type {describe_types(base_types)} became {describe_types(current_types)}",
            )

    def compare_enum(self, base: dict, current: dict, pointer: str) -> None:
        base_values = enum_values(base)
        current_values = enum_values(current)
        if self.is_request and current_values is not None:
            if base_values is None:
                self.report("enum-narrowed", pointer, "an enum now limits the accepted values")
            elif lost := missing_values(base_values, current_values):
                self.report("enum-narrowed", pointer, f"no longer accepts {lost}")
        if not self.is_request and base_values is not None:
            if current_values is None:
                self.report("enum-widened", pointer, "the enum was removed")
            elif gained := missing_values(current_values, base_values):
                self.report("enum-widened", pointer, f"may now return {gained}")

    def compare_bounds(self, base: dict, current: dict, pointer: str) -> None:
        for keyword in LOWER_BOUNDS + UPPER_BOUNDS:
            upper = keyword in UPPER_BOUNDS
            stricter = bound_is_stricter(base.get(keyword), current.get(keyword), upper)
            looser = bound_is_stricter(current.get(keyword), base.get(keyword), upper)
            if self.is_request and stricter:
                self.report(
                    "constraint-narrowed",
                    pointer,
                    f"{keyword} {base.get(keyword)} became {current.get(keyword)}",
                )
            if not self.is_request and looser:
                self.report(
                    "constraint-widened",
                    pointer,
                    f"{keyword} {base.get(keyword)} became {current.get(keyword)}",
                )
        base_unique = bool(base.get("uniqueItems"))
        current_unique = bool(current.get("uniqueItems"))
        if self.is_request and current_unique and not base_unique:
            self.report("constraint-narrowed", pointer, "uniqueItems was added")
        if not self.is_request and base_unique and not current_unique:
            self.report("constraint-widened", pointer, "uniqueItems was removed")
        if base.get("multipleOf") != current.get("multipleOf"):
            relevant = current if self.is_request else base
            if "multipleOf" in relevant:
                rule = "constraint-narrowed" if self.is_request else "constraint-widened"
                self.report(
                    rule,
                    pointer,
                    f"multipleOf {base.get('multipleOf')} became {current.get('multipleOf')}",
                )

    def compare_strings(self, base: dict, current: dict, pointer: str) -> None:
        for keyword, rule in (("pattern", "pattern-changed"), ("format", "format-changed")):
            base_value = base.get(keyword)
            current_value = current.get(keyword)
            if base_value == current_value:
                continue
            constrained = current_value if self.is_request else base_value
            if constrained is not None:
                self.report(rule, pointer, f"{keyword} {base_value!r} became {current_value!r}")

    def compare_default(self, base: dict, current: dict, pointer: str) -> None:
        if not self.is_request or "default" not in base:
            return
        if base.get("default") != current.get("default"):
            self.report(
                "default-changed",
                pointer,
                f"default {base['default']!r} became {current.get('default')!r}",
            )

    def compare_object(self, base: dict, current: dict, pointer: str, seen: frozenset) -> None:
        base_properties = base.get("properties", {})
        current_properties = current.get("properties", {})
        base_required = set(base.get("required", []))
        current_required = set(current.get("required", []))
        for name, schema in base_properties.items():
            child = f"{pointer}.{name}"
            if name not in current_properties:
                self.report("property-removed", child, "the property no longer exists")
                continue
            self.compare(schema, current_properties[name], child, seen)
        if self.is_request:
            for name in sorted(current_required - base_required):
                self.report(
                    "required-added", f"{pointer}.{name}", "clients that omit it are rejected"
                )
            if closes_additional_properties(base, current):
                self.report(
                    "additional-properties-closed", pointer, "extra properties are now rejected"
                )
        else:
            for name in sorted(base_required - current_required):
                if name in current_properties:
                    self.report(
                        "required-removed", f"{pointer}.{name}", "the property may be absent"
                    )
        base_extra = base.get("additionalProperties")
        current_extra = current.get("additionalProperties")
        if isinstance(base_extra, dict) and isinstance(current_extra, dict):
            self.compare(base_extra, current_extra, f"{pointer}.*", seen)

    def compare_items(self, base: dict, current: dict, pointer: str, seen: frozenset) -> None:
        if "items" in base and "items" in current:
            self.compare(base["items"], current["items"], f"{pointer}[]", seen)

    def compare_composition(self, base: dict, current: dict, pointer: str, seen: frozenset) -> None:
        for keyword in (*ALTERNATIVES, "allOf"):
            base_members = base.get(keyword)
            current_members = current.get(keyword)
            if base_members is None and current_members is None:
                continue
            if base_members is None or current_members is None:
                self.report("composition-changed", pointer, f"{keyword} was added or removed")
                continue
            pairs, removed, added = match_composition_members(base_members, current_members)
            for index, old, new in pairs:
                self.compare(old, new, f"{pointer}<{keyword} {index}>", seen)
            for members, was_added in ((removed, False), (added, True)):
                if not self.composition_member_breaks(keyword, was_added):
                    continue
                change = "added" if was_added else "removed"
                for member in members:
                    self.report(
                        "composition-changed", pointer,
                        f"{keyword} member {json.dumps(member, sort_keys=True)} was {change}",
                    )

    def composition_member_breaks(self, keyword: str, added: bool) -> bool:
        widens = added if keyword in ALTERNATIVES else not added
        return widens != self.is_request


def composition_identity(member: object) -> tuple[str, str | tuple[str, ...]] | None:
    if target := reference(member):
        return ("$ref", target)
    if isinstance(member, dict):
        member_type = member.get("type")
        if isinstance(member_type, str):
            return ("type", member_type)
        if isinstance(member_type, list) and all(isinstance(value, str) for value in member_type):
            return ("type", tuple(sorted(member_type)))
    return None


def match_composition_members(
    base: list, current: list
) -> tuple[list[tuple[int, object, object]], list, list]:
    remaining = list(current)
    unmatched = []
    pairs = []
    # Match equal members first so repeated types cannot pair the wrong branches.
    for index, old in enumerate(base):
        match = next((position for position, new in enumerate(remaining) if old == new), None)
        if match is None:
            unmatched.append((index, old))
        else:
            new = remaining.pop(match)
            pairs.append((index, old, new))
    removed = []
    for index, old in unmatched:
        identity = composition_identity(old)
        match = next(
            (position for position, new in enumerate(remaining)
             if identity is not None and composition_identity(new) == identity),
            None,
        )
        if match is None:
            removed.append(old)
        else:
            new = remaining.pop(match)
            pairs.append((index, old, new))
    return sorted(pairs, key=lambda pair: pair[0]), removed, remaining


def reference(node: object) -> str | None:
    if isinstance(node, dict) and isinstance(node.get("$ref"), str):
        return node["$ref"]
    return None


def resolve(document: dict, node: object) -> object:
    seen: set[str] = set()
    while (target := reference(node)) is not None:
        if target in seen:
            raise ConfigurationError(f"{target}: reference cycle")
        seen.add(target)
        node = pointer_target(document, target)
    return node


def pointer_target(document: dict, target: str) -> object:
    if not target.startswith("#/"):
        raise ConfigurationError(f"{target}: only local references are supported")
    node: object = document
    for token in target[2:].split("/"):
        token = token.replace("~1", "/").replace("~0", "~")
        if not isinstance(node, dict) or token not in node:
            raise ConfigurationError(f"{target}: unresolved reference")
        node = node[token]
    return node


def normalized_path(path: str) -> str:
    return PATH_PARAMETER.sub("{}", path)


def operations(document: dict, prefixes: tuple[str, ...]) -> dict:
    found = {}
    for path, item in document.get("paths", {}).items():
        if prefixes and not path.startswith(prefixes):
            continue
        item = resolve(document, item)
        for method in METHODS:
            operation = item.get(method)
            if isinstance(operation, dict):
                key = (method, normalized_path(path))
                found[key] = (f"{method.upper()} {path}", operation, (path, item))
    return found


def parameters(document: dict, path: str, item: dict, operation: dict) -> dict:
    """Merge path-item and operation parameters; key path parameters by template position."""
    path_names = [match[1:-1] for match in PATH_PARAMETER.findall(path)]
    merged = {}
    for source in (item.get("parameters", []), operation.get("parameters", [])):
        for parameter in source:
            parameter = resolve(document, parameter)
            merged[parameter_key(parameter, path_names)] = parameter
    return merged


def parameter_key(parameter: dict, path_names: list[str]) -> tuple:
    location = parameter.get("in", "")
    name = parameter.get("name", "")
    if location == "path" and name in path_names:
        return location, path_names.index(name)
    if location == "header":
        return location, name.lower()
    return location, name


def parameter_schema(parameter: dict) -> object:
    if "schema" in parameter:
        return parameter["schema"]
    for media in parameter.get("content", {}).values():
        return media.get("schema")
    return None


def describe_parameter(key: tuple, parameter: dict) -> str:
    return f"{key[0]} {parameter.get('name', '')}"


def security_options(document: dict, operation: dict) -> list[dict]:
    requirements = operation.get("security", document.get("security"))
    if not requirements:
        return [{}]
    return [
        {scheme: set(scopes) for scheme, scopes in requirement.items()}
        for requirement in requirements
    ]


def satisfies(sent: dict, required: dict) -> bool:
    return all(
        scheme in sent and scopes <= sent[scheme] for scheme, scopes in required.items()
    )


def describe_security(option: dict) -> str:
    if not option:
        return "no credential"
    return " and ".join(
        f"{scheme}{sorted(scopes) if scopes else ''}" for scheme, scopes in sorted(option.items())
    )


def schema_types(schema: dict) -> frozenset[str] | None:
    declared = schema.get("type")
    if declared is None:
        types = None
    elif isinstance(declared, list):
        types = set(declared)
    else:
        types = {declared}
    if types is not None and schema.get("nullable") is True:
        types.add("null")
    return None if types is None else frozenset(types)


def covers(wider: frozenset[str] | None, narrower: frozenset[str] | None) -> bool:
    if wider is None:
        return True
    if narrower is None:
        return False
    return all(kind in wider or (kind == "integer" and "number" in wider) for kind in narrower)


def describe_types(types: frozenset[str] | None) -> str:
    return "any" if types is None else "|".join(sorted(types))


def enum_values(schema: dict) -> list | None:
    if "const" in schema:
        return [schema["const"]]
    values = schema.get("enum")
    return None if values is None else list(values)


def missing_values(source: list, target: list) -> list:
    return [value for value in source if value not in target]


def bound_is_stricter(base: object, current: object, upper: bool) -> bool:
    if current is None or isinstance(current, bool):
        return False
    if base is None or isinstance(base, bool):
        return True
    return current < base if upper else current > base


def closes_additional_properties(base: dict, current: dict) -> bool:
    closed = current.get("additionalProperties") is False
    return closed and base.get("additionalProperties") is not False


def read_document(path: Path) -> dict:
    try:
        return parse_document(path.read_text(encoding="utf-8"), str(path))
    except OSError as error:
        raise ConfigurationError(f"{path}: cannot read document: {error}") from error


def parse_document(text: str, source: str) -> dict:
    try:
        document = json.loads(text)
    except json.JSONDecodeError as error:
        raise ConfigurationError(f"{source}: invalid JSON: {error}") from error
    if not isinstance(document, dict) or not isinstance(document.get("paths", {}), dict):
        raise ConfigurationError(f"{source}: not an OpenAPI document")
    return document


def read_revision(revision: str, path: Path) -> dict | None:
    """Return the document at a git revision, or None when the revision lacks the file."""
    if git("rev-parse", "--verify", "--quiet", f"{revision}^{{commit}}").returncode != 0:
        raise ConfigurationError(f"{revision}: unknown git revision")
    spec = f"{revision}:./{Path(os.path.relpath(path)).as_posix()}"
    shown = git("show", spec)
    if shown.returncode != 0:
        return None
    return parse_document(shown.stdout, spec)


def git(*arguments: str) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(
            ["git", *arguments], capture_output=True, text=True, check=False
        )
    except OSError as error:
        raise ConfigurationError(f"cannot run git: {error}") from error


def read_accepted(path: Path | None) -> list[Accepted]:
    if path is None:
        return []
    try:
        content = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ConfigurationError(f"{path}: cannot read accepted breaks: {error}") from error
    entries = content.get("accepted") if isinstance(content, dict) else None
    if not isinstance(entries, list):
        raise ConfigurationError(f"{path}: expected an object with an 'accepted' list")
    return [accepted_entry(path, index, entry) for index, entry in enumerate(entries)]


def accepted_entry(path: Path, index: int, entry: object) -> Accepted:
    location = f"{path}: accepted[{index}]"
    if not isinstance(entry, dict):
        raise ConfigurationError(f"{location}: expected an object")
    values = {}
    for field in ("rule", "location", "reason"):
        value = entry.get(field)
        if not isinstance(value, str) or not value.strip():
            raise ConfigurationError(f"{location}: {field!r} must be a non-empty string")
        values[field] = value
    if values["rule"] not in RULES:
        raise ConfigurationError(f"{location}: unknown rule {values['rule']!r}")
    return Accepted(**values)


def partition(breaks: list[Break], accepted: list[Accepted]) -> tuple[list[Break], list[Accepted]]:
    keys = {(entry.rule, entry.location) for entry in accepted}
    found = {(item.rule, item.location) for item in breaks}
    open_breaks = [item for item in breaks if (item.rule, item.location) not in keys]
    stale = [entry for entry in accepted if (entry.rule, entry.location) not in found]
    return open_breaks, stale


def rule_list() -> str:
    return "\n".join(f"  {rule}: {summary}" for rule, summary in RULES.items())


def parse_arguments(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        epilog=f"rules:\n{rule_list()}",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    base = parser.add_mutually_exclusive_group(required=True)
    base.add_argument("--base", type=Path, metavar="FILE", help="base OpenAPI JSON document")
    base.add_argument(
        "--base-revision",
        metavar="REV",
        help="git revision whose copy of --current is the base; a revision without the file "
        "has nothing to compare",
    )
    parser.add_argument(
        "--current", type=Path, required=True, metavar="FILE", help="current OpenAPI JSON document"
    )
    parser.add_argument(
        "--path-prefix",
        action="append",
        default=[],
        metavar="PREFIX",
        help="compare only paths that start with PREFIX; repeatable (default: every path)",
    )
    parser.add_argument(
        "--accepted",
        type=Path,
        metavar="FILE",
        help='intentional breaks as {"accepted": [{"rule", "location", "reason"}]}',
    )
    parser.add_argument(
        "--enforce",
        action="store_true",
        help="exit 1 when a break is not accepted (default: report only and exit 0)",
    )
    return parser.parse_args(list(argv))


def load_base(arguments: argparse.Namespace) -> dict | None:
    if arguments.base is not None:
        return read_document(arguments.base)
    return read_revision(arguments.base_revision, arguments.current)


def print_result(open_breaks: list[Break], stale: list[Accepted], enforce: bool) -> int:
    for entry in stale:
        print(f"note: accepted break no longer occurs: {entry.rule} {entry.location}")
    if not open_breaks:
        print("OpenAPI compatibility check passed.")
        return 0
    mode = "failed" if enforce else "found breaking changes (report only)"
    stream = sys.stderr if enforce else sys.stdout
    print(f"OpenAPI compatibility check {mode}:", file=stream)
    for item in open_breaks:
        print(f"- {item.render()}", file=stream)
    return 1 if enforce else 0


def main(argv: Sequence[str] = ()) -> int:
    arguments = parse_arguments(argv)
    try:
        current = read_document(arguments.current)
        accepted = read_accepted(arguments.accepted)
        base = load_base(arguments)
        if base is None:
            print(
                f"OpenAPI compatibility check skipped: {arguments.base_revision} has no "
                f"{arguments.current}."
            )
            return 0
        breaks = Comparison(base, current, arguments.path_prefix).run()
    except ConfigurationError as error:
        print(f"OpenAPI compatibility check cannot run: {error}", file=sys.stderr)
        return 2
    open_breaks, stale = partition(breaks, accepted)
    return print_result(open_breaks, stale, arguments.enforce)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
