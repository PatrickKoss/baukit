from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path

CHECK_PATH = Path(__file__).resolve().parent / "check-openapi-compatibility.py"
SPEC = importlib.util.spec_from_file_location("check_openapi_compatibility", CHECK_PATH)
assert SPEC is not None and SPEC.loader is not None
check = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check
SPEC.loader.exec_module(check)

JSON = "application/json"
BEARER = [{"bearer": []}]


def ref(name: str) -> dict:
    return {"$ref": f"#/components/schemas/{name}"}


def json_content(schema: dict) -> dict:
    return {JSON: {"schema": schema}}


def error() -> dict:
    return {"description": "Error", "content": json_content(ref("ErrorEnvelope"))}


BASE = {
    "openapi": "3.1.0",
    "info": {"title": "Items", "version": "1"},
    "paths": {
        "/v1/items": {
            "get": {
                "operationId": "listItems",
                "security": BEARER,
                "parameters": [
                    {
                        "name": "limit",
                        "in": "query",
                        "required": False,
                        "schema": {"type": "integer", "minimum": 1, "maximum": 100},
                    }
                ],
                "responses": {
                    "200": {
                        "description": "Items",
                        "content": json_content({"type": "array", "items": ref("Item")}),
                    },
                    "401": error(),
                },
            },
            "post": {
                "operationId": "createItem",
                "security": BEARER,
                "requestBody": {"required": True, "content": json_content(ref("SaveItem"))},
                "responses": {
                    "201": {"description": "Created", "content": json_content(ref("Item"))},
                    "400": error(),
                },
            },
        },
        "/v1/items/{id}": {
            "parameters": [
                {
                    "name": "id",
                    "in": "path",
                    "required": True,
                    "schema": {"type": "string", "format": "uuid"},
                }
            ],
            "get": {
                "operationId": "getItem",
                "responses": {
                    "200": {"description": "Item", "content": json_content(ref("Item"))},
                    "404": error(),
                },
            },
            "delete": {
                "operationId": "deleteItem",
                "responses": {"204": {"description": "Deleted"}, "404": error()},
            },
        },
        "/v1/health": {
            "get": {
                "operationId": "health",
                "responses": {
                    "200": {
                        "description": "Healthy",
                        "content": {"text/plain": {"schema": {"type": "string"}}},
                    }
                },
            }
        },
        "/internal/debug": {
            "get": {"operationId": "debug", "responses": {"200": {"description": "Debug"}}}
        },
    },
    "components": {
        "schemas": {
            "ErrorEnvelope": {"type": "object"},
            "Item": {
                "type": "object",
                "required": ["id", "name", "status"],
                "properties": {
                    "id": {"type": "string"},
                    "name": {"type": "string", "maxLength": 80},
                    "status": {"type": "string", "enum": ["open", "closed"]},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "parent": {"oneOf": [{"type": "null"}, ref("Item")]},
                },
            },
            "SaveItem": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "minLength": 1, "maxLength": 80, "pattern": "^\\S"},
                    "status": {"type": "string", "enum": ["open", "closed"], "default": "open"},
                    "count": {"type": "integer", "minimum": 0},
                },
            },
        },
        "securitySchemes": {"bearer": {"type": "http", "scheme": "bearer"}},
    },
}

LIST = BASE["paths"]["/v1/items"]["get"]
Mutation = Callable[[dict], None]


def operation(document: dict, path: str, method: str) -> dict:
    return document["paths"][path][method]


def schema(document: dict, name: str) -> dict:
    return document["components"]["schemas"][name]


def run(*arguments: str) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        status = check.main(arguments)
    return status, stdout.getvalue(), stderr.getvalue()


class Documents:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.base = root / "base.json"
        self.current = root / "current.json"
        self.accepted = root / "accepted.json"
        self.write(self.base, BASE)

    @staticmethod
    def write(path: Path, content: dict) -> None:
        path.write_text(json.dumps(content), encoding="utf-8")

    def changed(self, mutate: Mutation) -> None:
        document = copy.deepcopy(BASE)
        mutate(document)
        self.write(self.current, document)

    def compare(self, *extra: str) -> tuple[int, str, str]:
        return run("--base", str(self.base), "--current", str(self.current), *extra)


def set_value(target: Callable[[dict], dict], key: str, value: object) -> Mutation:
    def mutate(document: dict) -> None:
        target(document)[key] = value

    return mutate


def delete_key(target: Callable[[dict], dict], key: str) -> Mutation:
    def mutate(document: dict) -> None:
        del target(document)[key]

    return mutate


def list_items(document: dict) -> dict:
    return operation(document, "/v1/items", "get")


def create_item(document: dict) -> dict:
    return operation(document, "/v1/items", "post")


def get_item(document: dict) -> dict:
    return operation(document, "/v1/items/{id}", "get")


def delete_item(document: dict) -> dict:
    return operation(document, "/v1/items/{id}", "delete")


def item(document: dict) -> dict:
    return schema(document, "Item")


def item_property(name: str) -> Callable[[dict], dict]:
    return lambda document: item(document)["properties"][name]


def save_item(document: dict) -> dict:
    return schema(document, "SaveItem")


def save_property(name: str) -> Callable[[dict], dict]:
    return lambda document: save_item(document)["properties"][name]


def limit_parameter(document: dict) -> dict:
    return list_items(document)["parameters"][0]


def id_parameter(document: dict) -> dict:
    return document["paths"]["/v1/items/{id}"]["parameters"][0]["schema"]


def add_header_parameter(document: dict) -> None:
    list_items(document)["parameters"].append(
        {"name": "X-Tenant", "in": "header", "required": True, "schema": {"type": "string"}}
    )


def add_response_status(status: str) -> Mutation:
    return lambda document: delete_item(document)["responses"].update(
        {status: {"description": "Accepted"}}
    )


def rename_media_type(target: Callable[[dict], dict], old: str, new: str) -> Mutation:
    def mutate(document: dict) -> None:
        content = target(document)["content"]
        content[new] = content.pop(old)

    return mutate


LIST_RESPONSE = "GET /v1/items response 200 application/json"
CREATE_REQUEST = "POST /v1/items request application/json"

BREAKING_CHANGES: list[tuple[str, Mutation, str]] = [
    (
        "operation-removed",
        lambda document: document["paths"]["/v1/items"].pop("post"),
        "POST /v1/items",
    ),
    ("operation-id-changed", set_value(list_items, "operationId", "listAll"), "GET /v1/items"),
    (
        "security-changed",
        set_value(lambda document: operation(document, "/v1/health", "get"), "security", BEARER),
        "GET /v1/health security",
    ),
    (
        "security-changed",
        set_value(list_items, "security", [{"bearer": ["items:read"]}]),
        "GET /v1/items security",
    ),
    ("parameter-removed", set_value(list_items, "parameters", []), "GET /v1/items parameter query limit"),
    (
        "parameter-required",
        set_value(limit_parameter, "required", True),
        "GET /v1/items parameter query limit",
    ),
    ("parameter-required", add_header_parameter, "GET /v1/items parameter header X-Tenant"),
    (
        "request-body-required",
        set_value(
            delete_item, "requestBody", {"required": True, "content": json_content(ref("SaveItem"))}
        ),
        "DELETE /v1/items/{id} request",
    ),
    (
        "request-media-type-removed",
        rename_media_type(lambda document: create_item(document)["requestBody"], JSON, "text/csv"),
        CREATE_REQUEST,
    ),
    (
        "response-removed",
        lambda document: get_item(document)["responses"].pop("404"),
        "GET /v1/items/{id} response 404",
    ),
    ("success-status-added", add_response_status("202"), "DELETE /v1/items/{id} response 202"),
    (
        "response-media-type-removed",
        rename_media_type(
            lambda document: operation(document, "/v1/health", "get")["responses"]["200"],
            "text/plain",
            JSON,
        ),
        "GET /v1/health response 200 text/plain",
    ),
    ("type-changed", set_value(save_property("count"), "type", "string"), f"{CREATE_REQUEST} $.count"),
    (
        "type-changed",
        set_value(item_property("id"), "type", ["string", "null"]),
        f"{LIST_RESPONSE} $[].id",
    ),
    ("enum-narrowed", set_value(save_property("status"), "enum", ["open"]), f"{CREATE_REQUEST} $.status"),
    (
        "enum-widened",
        set_value(item_property("status"), "enum", ["open", "closed", "archived"]),
        f"{LIST_RESPONSE} $[].status",
    ),
    (
        "property-removed",
        lambda document: item(document)["properties"].pop("tags"),
        f"{LIST_RESPONSE} $[].tags",
    ),
    (
        "property-removed",
        lambda document: save_item(document)["properties"].pop("count"),
        f"{CREATE_REQUEST} $.count",
    ),
    (
        "required-added",
        set_value(save_item, "required", ["name", "status"]),
        f"{CREATE_REQUEST} $.status",
    ),
    ("required-removed", set_value(item, "required", ["id", "status"]), f"{LIST_RESPONSE} $[].name"),
    (
        "additional-properties-closed",
        set_value(save_item, "additionalProperties", False),
        f"{CREATE_REQUEST} $",
    ),
    ("constraint-narrowed", set_value(save_property("name"), "maxLength", 40), f"{CREATE_REQUEST} $.name"),
    ("constraint-narrowed", set_value(limit_parameter, "schema", {"type": "integer", "minimum": 5}), "GET /v1/items parameter query limit $"),
    ("constraint-widened", set_value(item_property("name"), "maxLength", 200), f"{LIST_RESPONSE} $[].name"),
    ("constraint-widened", delete_key(item_property("name"), "maxLength"), f"{LIST_RESPONSE} $[].name"),
    ("pattern-changed", set_value(save_property("name"), "pattern", "^[a-z]"), f"{CREATE_REQUEST} $.name"),
    ("format-changed", set_value(id_parameter, "format", "uri"), "GET /v1/items/{id} parameter path id $"),
    ("default-changed", set_value(save_property("status"), "default", "closed"), f"{CREATE_REQUEST} $.status"),
    (
        "composition-changed",
        set_value(item_property("parent"), "oneOf", [{"type": "null"}, ref("Item"), {"type": "string"}]),
        f"{LIST_RESPONSE} $[].parent",
    ),
]


def additive_change(document: dict) -> None:
    document["paths"]["/v1/tags"] = {
        "get": {"operationId": "listTags", "responses": {"200": {"description": "Tags"}}}
    }
    list_items(document)["parameters"].append(
        {"name": "cursor", "in": "query", "required": False, "schema": {"type": "string"}}
    )
    list_items(document)["security"] = [{}, {"bearer": []}]
    get_item(document)["responses"]["429"] = error()
    get_item(document)["responses"]["200"]["content"]["application/xml"] = {
        "schema": {"type": "string"}
    }
    save_item(document)["properties"]["note"] = {"type": "string"}
    save_property("status")(document)["enum"].append("archived")
    save_property("name")(document)["maxLength"] = 200
    save_property("count")(document)["type"] = "number"
    item(document)["properties"]["createdAt"] = {"type": "string", "format": "date-time"}
    create_item(document)["requestBody"]["content"]["application/merge-patch+json"] = {
        "schema": ref("SaveItem")
    }
    document["paths"]["/v1/items/{itemId}"] = document["paths"].pop("/v1/items/{id}")
    document["paths"]["/v1/items/{itemId}"]["parameters"][0]["name"] = "itemId"


class CompatibilityRuleTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.documents = Documents(Path(directory.name))

    def test_every_listed_breaking_change_fails_in_enforce_mode(self) -> None:
        covered = set()
        for rule, mutate, location in BREAKING_CHANGES:
            with self.subTest(rule=rule, location=location):
                self.documents.changed(mutate)

                status, _, stderr = self.documents.compare("--enforce")

                self.assertEqual(status, 1, stderr)
                self.assertIn(f"- {rule} {location}: ", stderr)
                covered.add(rule)
        self.assertEqual(covered, set(check.RULES))

    def test_an_additive_change_passes(self) -> None:
        self.documents.changed(additive_change)

        status, stdout, stderr = self.documents.compare("--enforce")

        self.assertEqual(status, 0, stdout + stderr)
        self.assertEqual(stdout, "OpenAPI compatibility check passed.\n")

    def test_recursive_schemas_compare_without_looping(self) -> None:
        self.documents.changed(lambda document: None)

        status, stdout, _ = self.documents.compare("--enforce")

        self.assertEqual(status, 0)
        self.assertIn("passed", stdout)

    def test_reordered_nullable_reference_alternatives_pass(self) -> None:
        for keyword in check.ALTERNATIVES:
            with self.subTest(keyword=keyword):
                base = copy.deepcopy(BASE)
                parent = item_property("parent")(base)
                parent[keyword] = parent.pop("oneOf") if keyword != "oneOf" else parent["oneOf"]
                self.documents.write(self.documents.base, base)
                current = copy.deepcopy(base)
                item_property("parent")(current)[keyword].reverse()
                self.documents.write(self.documents.current, current)
                status, stdout, stderr = self.documents.compare("--enforce")
                self.assertEqual(status, 0, stdout + stderr)
                self.assertEqual(stderr, "")

    def test_reordered_nullable_types_and_primitive_alternatives_pass(self) -> None:
        schemas = [
            {"type": ["string", "null"]},
            *({keyword: [{"type": "string"}, {"type": "null"}]} for keyword in check.ALTERNATIVES),
        ]
        for schema in schemas:
            for target in (item_property("name"), save_property("name")):
                with self.subTest(schema=schema, target=target):
                    base = copy.deepcopy(BASE)
                    target(base).clear()
                    target(base).update(schema)
                    current = copy.deepcopy(base)
                    keyword = next(iter(schema))
                    target(current)[keyword].reverse()
                    self.documents.write(self.documents.base, base)
                    self.documents.write(self.documents.current, current)
                    status, stdout, stderr = self.documents.compare("--enforce")
                    self.assertEqual(status, 0, stdout + stderr)
                    self.assertEqual(stderr, "")

    def test_removed_request_alternative_is_reported_as_a_member_removal(self) -> None:
        for keyword in check.ALTERNATIVES:
            with self.subTest(keyword=keyword):
                base = copy.deepcopy(BASE)
                save_property("count")(base).clear()
                save_property("count")(base)[keyword] = [{"type": "integer"}, {"type": "null"}]
                self.documents.write(self.documents.base, base)
                current = copy.deepcopy(base)
                save_property("count")(current)[keyword].pop()
                self.documents.write(self.documents.current, current)
                status, _, stderr = self.documents.compare("--enforce")
                self.assertEqual(status, 1, stderr)
                self.assertIn(f'{keyword} member {{"type": "null"}} was removed', stderr)

    def test_reordering_still_checks_changes_inside_matched_references(self) -> None:
        base = copy.deepcopy(BASE)
        list_items(base)["responses"]["200"]["content"][JSON]["schema"] = {
            "oneOf": [{"type": "null"}, ref("Item")]
        }
        self.documents.write(self.documents.base, base)
        current = copy.deepcopy(base)
        list_items(current)["responses"]["200"]["content"][JSON]["schema"]["oneOf"].reverse()
        item_property("name")(current)["maxLength"] = 200
        self.documents.write(self.documents.current, current)
        status, _, stderr = self.documents.compare("--enforce")
        self.assertEqual(status, 1, stderr)
        self.assertIn(f"constraint-widened {LIST_RESPONSE} $<oneOf 1>.name", stderr)
        self.assertNotIn("type-changed", stderr)

    def test_reordered_primitive_members_still_detect_changed_constraints(self) -> None:
        base = copy.deepcopy(BASE)
        save_property("count")(base).clear()
        save_property("count")(base)["anyOf"] = [{"type": "integer", "minimum": 0}, {"type": "null"}]
        self.documents.write(self.documents.base, base)
        current = copy.deepcopy(base)
        save_property("count")(current)["anyOf"] = [{"type": "null"}, {"type": "integer", "minimum": 10}]
        self.documents.write(self.documents.current, current)
        status, _, stderr = self.documents.compare("--enforce")
        self.assertEqual(status, 1, stderr)
        self.assertIn("constraint-narrowed", stderr)

    def test_equal_count_replacement_reports_added_response_member(self) -> None:
        self.documents.changed(set_value(item_property("parent"), "oneOf", [ref("Item"), {"type": "string"}]))
        status, _, stderr = self.documents.compare("--enforce")
        self.assertEqual(status, 1, stderr)
        self.assertIn('oneOf member {"type": "string"} was added', stderr)

    def test_structural_matching_preserves_duplicate_type_and_untyped_members(self) -> None:
        members = [{"type": "string", "enum": ["a"]}, {"type": "string", "enum": ["b"]}, {"const": "c"}]
        pairs, removed, added = check.match_composition_members(members, list(reversed(members)))
        self.assertEqual([(old, new) for _, old, new in pairs], [(member, member) for member in members])
        self.assertEqual(removed, [])
        self.assertEqual(added, [])

    def test_request_additions_and_response_removals_keep_their_compatibility_direction(self) -> None:
        base = copy.deepcopy(BASE)
        save_property("count")(base).clear()
        save_property("count")(base)["anyOf"] = [{"type": "integer"}]
        self.documents.write(self.documents.base, base)
        current = copy.deepcopy(base)
        save_property("count")(current)["anyOf"].append({"type": "null"})
        item_property("parent")(current)["oneOf"] = [ref("Item")]
        self.documents.write(self.documents.current, current)
        status, stdout, stderr = self.documents.compare("--enforce")
        self.assertEqual(status, 0, stdout + stderr)

    def test_report_only_prints_breaks_and_exits_zero(self) -> None:
        self.documents.changed(delete_key(lambda document: document["paths"]["/v1/items"], "post"))

        status, stdout, _ = self.documents.compare()

        self.assertEqual(status, 0)
        self.assertIn("found breaking changes (report only)", stdout)
        self.assertIn("- operation-removed POST /v1/items: ", stdout)

    def test_path_prefix_limits_the_comparison(self) -> None:
        self.documents.changed(lambda document: document["paths"].pop("/internal/debug"))

        status, _, _ = self.documents.compare("--enforce", "--path-prefix", "/v1/")

        self.assertEqual(status, 0)


class AcceptedBreakTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.documents = Documents(Path(directory.name))
        self.documents.changed(set_value(list_items, "operationId", "listAll"))

    def accept(self, *entries: dict) -> None:
        Documents.write(self.documents.accepted, {"accepted": list(entries)})

    def test_an_accepted_break_passes_and_a_stale_entry_is_noted(self) -> None:
        self.accept(
            {
                "rule": "operation-id-changed",
                "location": "GET /v1/items",
                "reason": "Clients generate from the new name in the same release.",
            },
            {"rule": "operation-removed", "location": "GET /v1/gone", "reason": "Removed earlier."},
        )

        status, stdout, stderr = self.documents.compare(
            "--enforce", "--accepted", str(self.documents.accepted)
        )

        self.assertEqual(status, 0, stderr)
        self.assertIn("note: accepted break no longer occurs: operation-removed GET /v1/gone", stdout)
        self.assertIn("passed", stdout)

    def test_rejects_invalid_accepted_entries(self) -> None:
        cases = {
            "'reason' must be a non-empty string": {
                "rule": "operation-id-changed",
                "location": "GET /v1/items",
                "reason": " ",
            },
            "unknown rule": {"rule": "renamed", "location": "GET /v1/items", "reason": "Why."},
        }
        for message, entry in cases.items():
            with self.subTest(message=message):
                self.accept(entry)

                status, _, stderr = self.documents.compare(
                    "--accepted", str(self.documents.accepted)
                )

                self.assertEqual(status, 2)
                self.assertIn(message, stderr)


class BaseRevisionTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git("init", "--quiet")
        self.schema = self.root / "backend" / "openapi.json"
        self.schema.parent.mkdir()
        previous = Path.cwd()
        os.chdir(self.root)
        self.addCleanup(os.chdir, previous)

    def git(self, *arguments: str) -> None:
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                *arguments,
            ],
            cwd=self.root,
            check=True,
            capture_output=True,
        )

    def commit_schema(self, document: dict) -> None:
        Documents.write(self.schema, document)
        self.git("add", "backend/openapi.json")
        self.git("commit", "--quiet", "-m", "schema")

    def test_compares_against_the_file_at_a_revision(self) -> None:
        self.commit_schema(BASE)
        changed = copy.deepcopy(BASE)
        del changed["paths"]["/v1/health"]
        Documents.write(self.schema, changed)

        status, _, stderr = run(
            "--base-revision", "HEAD", "--current", "backend/openapi.json", "--enforce"
        )

        self.assertEqual(status, 1)
        self.assertIn("- operation-removed GET /v1/health: ", stderr)

    def test_a_revision_without_the_file_has_nothing_to_compare(self) -> None:
        (self.root / "README").write_text("start\n", encoding="utf-8")
        self.git("add", "README")
        self.git("commit", "--quiet", "-m", "start")
        Documents.write(self.schema, BASE)

        status, stdout, _ = run("--base-revision", "HEAD", "--current", "backend/openapi.json")

        self.assertEqual(status, 0)
        self.assertIn("skipped", stdout)

    def test_rejects_an_unknown_revision(self) -> None:
        self.commit_schema(BASE)

        status, _, stderr = run("--base-revision", "missing", "--current", "backend/openapi.json")

        self.assertEqual(status, 2)
        self.assertIn("unknown git revision", stderr)


if __name__ == "__main__":
    unittest.main()
