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
import importlib.util
import json
import re
import shutil
import sys
from collections.abc import Iterable
from pathlib import Path
from types import ModuleType

HELM_YAML = Path(__file__).resolve().parents[1] / "lint" / "helm_yaml.py"


def load_helm_yaml() -> ModuleType:
    spec = importlib.util.spec_from_file_location("helm_yaml", HELM_YAML)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {HELM_YAML}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


helm_yaml = load_helm_yaml()

STAMP_NAME = ".inputs.sha256"
CRD_KIND = "CustomResourceDefinition"
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


class GenerationError(Exception):
    pass


def inputs_digest(paths: Iterable[Path]) -> str:
    digest = hashlib.sha256(Path(__file__).read_bytes())
    digest.update(Path(helm_yaml.__file__).read_bytes())
    for content_hash in sorted(hashlib.sha256(path.read_bytes()).hexdigest() for path in paths):
        digest.update(content_hash.encode())
    return digest.hexdigest()


def crd_documents(paths: list[Path]) -> Iterable[tuple[str, str]]:
    """Yield (source, text) for each YAML document whose kind is a CRD."""
    for path in paths:
        for document in helm_yaml.split_documents(path.read_text()):
            if CRD_KIND_LINE.search(document):
                yield str(path), document


def load_crds(paths: list[Path]) -> list[tuple[str, dict]]:
    """Return (source, crd) pairs."""
    documents = list(crd_documents(paths))
    try:
        parsed = helm_yaml.parse_documents([text for _, text in documents])
    except helm_yaml.YamlError as error:
        raise GenerationError(str(error)) from error
    return [(source, crd) for (source, _), crd in zip(documents, parsed, strict=True)]


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
