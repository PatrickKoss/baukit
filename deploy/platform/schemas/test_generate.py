from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

GENERATE_PATH = Path(__file__).resolve().parent / "generate.py"
SPEC = importlib.util.spec_from_file_location("generate_crd_schemas", GENERATE_PATH)
assert SPEC is not None and SPEC.loader is not None
generate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = generate
SPEC.loader.exec_module(generate)

WIDGET_CRD = """\
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: widgets.example.com
spec:
  group: example.com
  names:
    kind: Widget
    plural: widgets
  scope: Namespaced
  versions:
    - name: v1
      served: true
      storage: true
      schema:
        openAPIV3Schema:
          type: object
          properties:
            spec:
              type: object
              properties:
                size:
                  type: integer
"""
CONFIG_MAP = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: unrelated\n"
KUBERNETES_DEFINITIONS = {
    "definitions": {
        generate.CRD_DEFINITION: {
            "properties": {
                "metadata": {"$ref": "#/definitions/meta.ObjectMeta"},
                "spec": {"$ref": "#/definitions/crd.Spec"},
            }
        },
        "crd.Spec": {"properties": {"group": {"type": "string"}}},
        "meta.ObjectMeta": {"properties": {"name": {"type": "string"}}},
        "unused.Thing": {"properties": {"x": {"type": "string"}}},
    }
}


def crd(group: str, kind: str, spec_properties: dict) -> dict:
    return {
        "metadata": {"name": f"{kind.lower()}s.{group}"},
        "spec": {
            "group": group,
            "names": {"kind": kind},
            "versions": [
                {
                    "name": "v1",
                    "schema": {
                        "openAPIV3Schema": {
                            "type": "object",
                            "properties": {
                                "spec": {"type": "object", "properties": spec_properties}
                            },
                        }
                    },
                }
            ],
        },
    }


class ConvertTest(unittest.TestCase):
    def test_objects_with_properties_are_closed(self) -> None:
        schema = generate.convert({"type": "object", "properties": {"a": {"type": "string"}}})
        self.assertEqual(schema["additionalProperties"], False)

    def test_preserve_unknown_fields_stays_open_but_children_close(self) -> None:
        schema = generate.convert(
            {
                "x-kubernetes-preserve-unknown-fields": True,
                "properties": {"child": {"properties": {"a": {"type": "string"}}}},
            }
        )
        self.assertNotIn("additionalProperties", schema)
        self.assertEqual(schema["properties"]["child"]["additionalProperties"], False)

    def test_existing_additional_properties_are_kept_and_converted(self) -> None:
        schema = generate.convert(
            {"properties": {}, "additionalProperties": {"properties": {"a": {}}}}
        )
        self.assertEqual(schema["additionalProperties"]["additionalProperties"], False)

    def test_combinator_branches_are_not_closed(self) -> None:
        schema = generate.convert({"anyOf": [{"properties": {"a": {}}}, {"required": ["b"]}]})
        self.assertNotIn("additionalProperties", schema["anyOf"][0])

    def test_items_are_closed(self) -> None:
        schema = generate.convert({"type": "array", "items": {"properties": {"a": {}}}})
        self.assertEqual(schema["items"]["additionalProperties"], False)

    def test_a_field_named_properties_is_a_schema_not_a_map(self) -> None:
        schema = generate.convert({"properties": {"properties": {"type": "string"}}})
        self.assertEqual(schema["properties"]["properties"], {"type": "string"})

    def test_int_or_string_accepts_both(self) -> None:
        schema = generate.convert({"x-kubernetes-int-or-string": True})
        self.assertEqual(schema["oneOf"], [{"type": "string"}, {"type": "integer"}])

    def test_nullable_allows_null(self) -> None:
        schema = generate.convert({"type": "string", "nullable": True})
        self.assertEqual(schema["type"], ["string", "null"])


class SchemaCollectionTest(unittest.TestCase):
    def test_file_name_follows_the_kubeconform_template(self) -> None:
        schemas = generate.collect_schemas([("a", crd("example.com", "Widget", {}))])
        self.assertEqual(list(schemas), ["widget_v1.json"])
        root = schemas["widget_v1.json"]
        self.assertEqual(root["additionalProperties"], False)
        self.assertEqual(set(root["properties"]), {"apiVersion", "kind", "metadata", "spec"})

    def test_identical_duplicates_are_merged(self) -> None:
        widget = crd("example.com", "Widget", {})
        self.assertEqual(len(generate.collect_schemas([("a", widget), ("b", widget)])), 1)

    def test_differing_duplicates_fail(self) -> None:
        with self.assertRaisesRegex(generate.GenerationError, "differs between a and b"):
            generate.collect_schemas(
                [
                    ("a", crd("example.com", "Widget", {})),
                    ("b", crd("example.com", "Widget", {"size": {"type": "integer"}})),
                ]
            )

    def test_same_kind_in_two_groups_fails(self) -> None:
        with self.assertRaisesRegex(generate.GenerationError, "collides with group"):
            generate.collect_schemas(
                [("a", crd("example.com", "Cluster", {})), ("b", crd("other.io", "Cluster", {}))]
            )

    def test_version_without_schema_fails(self) -> None:
        broken = crd("example.com", "Widget", {})
        del broken["spec"]["versions"][0]["schema"]
        with self.assertRaisesRegex(generate.GenerationError, "no openAPIV3Schema"):
            generate.collect_schemas([("a", broken)])

    def test_crd_schema_bundles_only_referenced_definitions(self) -> None:
        schema = generate.crd_schema(KUBERNETES_DEFINITIONS)
        self.assertEqual(
            sorted(schema["definitions"]),
            sorted([generate.CRD_DEFINITION, "crd.Spec", "meta.ObjectMeta"]),
        )
        self.assertEqual(schema["definitions"]["crd.Spec"]["additionalProperties"], False)

    def test_crd_schema_with_dangling_reference_fails(self) -> None:
        definitions = json.loads(json.dumps(KUBERNETES_DEFINITIONS))
        del definitions["definitions"]["crd.Spec"]
        with self.assertRaisesRegex(generate.GenerationError, "lack crd.Spec"):
            generate.crd_schema(definitions)


class GenerateTest(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.definitions = self.root / "definitions.json"
        self.definitions.write_text(json.dumps(KUBERNETES_DEFINITIONS))
        self.manifest = self.root / "manifest.yaml"
        self.manifest.write_text(f"{CONFIG_MAP}---\n{WIDGET_CRD}")

    def snapshot(self, output: Path) -> dict[str, bytes]:
        return {path.name: path.read_bytes() for path in sorted(output.iterdir())}

    def test_writes_one_schema_per_crd_version_and_the_crd_schema(self) -> None:
        output = self.root / "out"
        self.assertEqual(generate.generate(output, self.definitions, [self.manifest]), 2)
        self.assertEqual(
            sorted(self.snapshot(output)),
            [generate.STAMP_NAME, "customresourcedefinition_v1.json", "widget_v1.json"],
        )
        widget = json.loads((output / "widget_v1.json").read_text())
        self.assertEqual(widget["properties"]["spec"]["additionalProperties"], False)

    def test_output_is_deterministic(self) -> None:
        first, second = self.root / "first", self.root / "second"
        generate.generate(first, self.definitions, [self.manifest])
        generate.generate(second, self.definitions, [self.manifest])
        self.assertEqual(self.snapshot(first), self.snapshot(second))

    def test_unchanged_inputs_reuse_the_cache_and_changed_inputs_regenerate(self) -> None:
        output = self.root / "out"
        generate.generate(output, self.definitions, [self.manifest])
        (output / "stale_v1.json").write_text("{}")
        generate.generate(output, self.definitions, [self.manifest])
        self.assertTrue((output / "stale_v1.json").exists())

        self.manifest.write_text(WIDGET_CRD.replace("integer", "string"))
        generate.generate(output, self.definitions, [self.manifest])
        self.assertFalse((output / "stale_v1.json").exists())
        widget = json.loads((output / "widget_v1.json").read_text())
        size = widget["properties"]["spec"]["properties"]["size"]
        self.assertEqual(size["type"], "string")

    def test_unparseable_yaml_fails(self) -> None:
        self.manifest.write_text("kind: CustomResourceDefinition\nspec: [unclosed\n")
        with self.assertRaisesRegex(generate.GenerationError, "could not parse"):
            generate.generate(self.root / "out", self.definitions, [self.manifest])

    def test_main_reports_errors_with_exit_code_one(self) -> None:
        self.manifest.write_text("kind: CustomResourceDefinition\nspec: [unclosed\n")
        argv = ["--output", str(self.root / "out"), "--kubernetes-definitions"]
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            code = generate.main([*argv, str(self.definitions), str(self.manifest)])
        self.assertEqual(code, 1)
        self.assertIn("ERROR: helm could not parse", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
