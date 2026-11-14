#!/usr/bin/env python3
"""Convert every CustomResourceDefinition in the given manifests into kubeconform JSON schemas.

The output directory holds one `{kind}_{version}.json` per served CRD version, matching the
kubeconform schema location template `{{ .ResourceKind }}_{{ .ResourceAPIVersion }}.json`.
Objects that declare properties get `additionalProperties: false`, so `kubeconform -strict`
rejects misspelled fields the same way the API server prunes them.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Iterable
from pathlib import Path

SOURCE_MARKER = "# crd-source "
STAMP_NAME = ".inputs.sha256"
CRD_KIND = "CustomResourceDefinition"
DOCUMENT_SEPARATOR = re.compile(r"^---[ \t]*$", re.MULTILINE)
CRD_KIND_LINE = re.compile(r"^kind:[ \t]*['\"]?CustomResourceDefinition['\"]?[ \t]*$", re.MULTILINE)
COMBINATORS = ("allOf", "anyOf", "oneOf")
SCHEMA_MAPS = ("properties", "patternProperties")
CRD_DEFINITION = "io.k8s.apiextensions-apiserver.pkg.apis.apiextensions.v1.CustomResourceDefinition"
CRD_SCHEMA_FILE = "customresourcedefinition_v1.json"
LOCAL_REF_PREFIX = "#/definitions/"
ROOT_PROPERTIES = {
    "apiVersion": {"type": "string"},
    "kind": {"type": "string"},
    "metadata": {"type": "object"},
}

CONVERTER_CHART = "apiVersion: v2\nname: crd-json\nversion: 0.0.0\n"
CONVERTER_TEMPLATE = """\
{{- range $path, $_ := .Files.Glob "files/*.yaml" }}
{{- range $doc := regexSplit "(?m)^---[ \\t]*$" ($.Files.Get $path) -1 }}
{{- $obj := fromYaml $doc }}
{{- if hasKey $obj "Error" }}{{ fail (printf "%s: %s" $path $obj.Error) }}{{ end }}
{{- if eq (toString (get $obj "kind")) "CustomResourceDefinition" }}
---
# crd-source {{ $path }}
{{ toJson $obj }}
{{- end }}
{{- end }}
{{- end }}
"""


class GenerationError(Exception):
    pass


def inputs_digest(paths: Iterable[Path]) -> str:
    digest = hashlib.sha256(Path(__file__).read_bytes())
    for content_hash in sorted(hashlib.sha256(path.read_bytes()).hexdigest() for path in paths):
        digest.update(content_hash.encode())
    return digest.hexdigest()


def crd_documents(paths: list[Path]) -> Iterable[tuple[str, str, str]]:
    """Yield (chart file name, source, text) for each YAML document whose kind is a CRD."""
    index = 0
    for path in paths:
        for document in DOCUMENT_SEPARATOR.split(path.read_text()):
            if CRD_KIND_LINE.search(document):
                yield f"files/{index:05d}.yaml", str(path), document
                index += 1


def load_crds(paths: list[Path]) -> list[tuple[str, dict]]:
    """Return (source, crd) pairs, parsing YAML with helm's fromYaml since stdlib has no YAML."""
    helm = shutil.which("helm")
    if helm is None:
        raise GenerationError("helm is required to parse YAML manifests")
    with tempfile.TemporaryDirectory() as chart_dir:
        chart = Path(chart_dir)
        (chart / "templates").mkdir()
        (chart / "files").mkdir()
        (chart / "Chart.yaml").write_text(CONVERTER_CHART)
        (chart / "templates" / "crds.yaml").write_text(CONVERTER_TEMPLATE)
        sources = {}
        for name, source, document in crd_documents(paths):
            sources[name] = source
            (chart / name).write_text(document)
        result = subprocess.run(  # noqa: S603 -- argv is the helm found by shutil.which plus fixed flags, no shell
            [helm, "template", "crd-json", str(chart)],
            capture_output=True,
            text=True,
            check=False,
        )
    if result.returncode != 0:
        raise GenerationError(f"helm could not parse the manifests:\n{result.stderr.strip()}")
    crds = []
    source = ""
    for line in result.stdout.splitlines():
        if line.startswith(SOURCE_MARKER):
            source = sources[line.removeprefix(SOURCE_MARKER)]
        elif line.startswith("{"):
            crds.append((source, json.loads(line)))
    return crds


def convert(node: object, *, allow_closing: bool = True) -> object:
    if isinstance(node, list):
        return [convert(item, allow_closing=allow_closing) for item in node]
    if not isinstance(node, dict):
        return node
    schema = {}
    for key, value in node.items():
        if key in SCHEMA_MAPS and isinstance(value, dict):
            schema[key] = {name: convert(child) for name, child in value.items()}
        elif key in COMBINATORS and isinstance(value, list):
            schema[key] = [convert(child, allow_closing=False) for child in value]
        elif key in ("items", "additionalProperties", "not"):
            schema[key] = convert(value)
        else:
            schema[key] = copy.deepcopy(value)
    if schema.get("nullable") is True and isinstance(schema.get("type"), str):
        schema["type"] = [schema["type"], "null"]
    if schema.get("x-kubernetes-int-or-string") is True and not any(
        key in schema for key in COMBINATORS
    ):
        schema["oneOf"] = [{"type": "string"}, {"type": "integer"}]
    closes = (
        allow_closing
        and "properties" in schema
        and "additionalProperties" not in schema
        and schema.get("x-kubernetes-preserve-unknown-fields") is not True
    )
    if closes:
        schema["additionalProperties"] = False
    return schema


def version_schemas(crd: dict) -> dict[tuple[str, str], tuple[str, dict]]:
    spec = crd.get("spec", {})
    group = spec.get("group", "")
    kind = spec.get("names", {}).get("kind", "")
    if not group or not kind:
        raise GenerationError(f"CRD {crd.get('metadata', {}).get('name')} lacks group or kind")
    schemas = {}
    for version in spec.get("versions", []):
        openapi = version.get("schema", {}).get("openAPIV3Schema")
        if openapi is None:
            raise GenerationError(f"{group}/{version.get('name')} {kind} has no openAPIV3Schema")
        root = convert(openapi)
        if not isinstance(root, dict):
            raise GenerationError(f"{group}/{version.get('name')} {kind} schema is not an object")
        properties = root.setdefault("properties", {})
        for name, default in ROOT_PROPERTIES.items():
            properties.setdefault(name, default)
        if root.get("x-kubernetes-preserve-unknown-fields") is not True:
            root.setdefault("additionalProperties", False)
        file_name = f"{kind.lower()}_{version['name']}.json"
        schemas[(file_name, group)] = (f"{group}/{version['name']} {kind}", root)
    return schemas


def referenced_definitions(node: object) -> Iterable[str]:
    if isinstance(node, dict):
        reference = node.get("$ref")
        if isinstance(reference, str) and reference.startswith(LOCAL_REF_PREFIX):
            yield reference.removeprefix(LOCAL_REF_PREFIX)
        for value in node.values():
            yield from referenced_definitions(value)
    elif isinstance(node, list):
        for item in node:
            yield from referenced_definitions(item)


def crd_schema(kubernetes_definitions: dict) -> dict:
    """Bundle the CustomResourceDefinition schema with the definitions it references, closed."""
    definitions = kubernetes_definitions.get("definitions", {})
    needed = {CRD_DEFINITION}
    pending = [CRD_DEFINITION]
    while pending:
        name = pending.pop()
        if name not in definitions:
            raise GenerationError(f"Kubernetes definitions lack {name}")
        for reference in referenced_definitions(definitions[name]):
            if reference not in needed:
                needed.add(reference)
                pending.append(reference)
    root = convert(definitions[CRD_DEFINITION])
    if not isinstance(root, dict):
        raise GenerationError(f"{CRD_DEFINITION} is not an object schema")
    root["definitions"] = {name: convert(definitions[name]) for name in sorted(needed)}
    return root


def collect_schemas(crds: list[tuple[str, dict]]) -> dict[str, dict]:
    chosen: dict[str, tuple[str, str, dict]] = {}
    problems = []
    for source, crd in sorted(crds, key=lambda item: (item[1]["metadata"]["name"], item[0])):
        for (file_name, group), (label, schema) in version_schemas(crd).items():
            previous = chosen.get(file_name)
            if previous is None:
                chosen[file_name] = (group, source, schema)
            elif previous[0] != group:
                problems.append(f"{file_name}: {label} collides with group {previous[0]}")
            elif previous[2] != schema:
                problems.append(f"{file_name}: {label} differs between {previous[1]} and {source}")
    if problems:
        raise GenerationError("conflicting CRD schemas:\n  " + "\n  ".join(sorted(set(problems))))
    return {file_name: schema for file_name, (_, _, schema) in chosen.items()}


def write_schemas(output: Path, schemas: dict[str, dict], stamp: str) -> None:
    staging = output.with_name(output.name + ".tmp")
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    for file_name in sorted(schemas):
        text = json.dumps(schemas[file_name], indent=2, sort_keys=True) + "\n"
        (staging / file_name).write_text(text)
    (staging / STAMP_NAME).write_text(stamp + "\n")
    shutil.rmtree(output, ignore_errors=True)
    staging.rename(output)


def generate(output: Path, kubernetes_definitions: Path, inputs: list[Path]) -> int:
    stamp = inputs_digest([kubernetes_definitions, *inputs])
    stamp_file = output / STAMP_NAME
    if stamp_file.is_file() and stamp_file.read_text().strip() == stamp:
        return len([path for path in output.glob("*.json")])
    schemas = collect_schemas(load_crds(inputs))
    schemas[CRD_SCHEMA_FILE] = crd_schema(json.loads(kubernetes_definitions.read_text()))
    write_schemas(output, schemas, stamp)
    return len(schemas)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--kubernetes-definitions",
        type=Path,
        required=True,
        help="kubernetes-json-schema _definitions.json, the source of the CRD schema",
    )
    parser.add_argument("inputs", type=Path, nargs="+")
    args = parser.parse_args(argv)
    try:
        count = generate(args.output, args.kubernetes_definitions, args.inputs)
    except (GenerationError, OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print(f"CRD schemas: {count} in {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
