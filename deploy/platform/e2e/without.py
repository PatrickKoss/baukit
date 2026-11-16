"""Drops the named Flux Kustomizations from a stream of JSON objects and from every dependsOn list.

Usage: without.py NAME... < objects.json > list.json. The input is what `kubectl create -o json`
prints, one object after another; the output is a v1 List. Exits 2 when a NAME is not in the input.
"""

import json
import sys

KUSTOMIZATION_KIND = "Kustomization"


def is_kustomization(item):
    return item.get("kind") == KUSTOMIZATION_KIND and item.get("apiVersion", "").startswith(
        "kustomize.toolkit.fluxcd.io/"
    )


def read_objects(text):
    decoder = json.JSONDecoder()
    objects, position = [], 0
    while text[position:].strip():
        position += len(text[position:]) - len(text[position:].lstrip())
        item, position = decoder.raw_decode(text, position)
        objects.append(item)
    return objects


def without(items, names):
    present = {item["metadata"]["name"] for item in items if is_kustomization(item)}
    unknown = sorted(set(names) - present)
    if unknown:
        raise KeyError(", ".join(unknown))
    kept = []
    for item in items:
        if is_kustomization(item) and item["metadata"]["name"] in names:
            continue
        if is_kustomization(item) and "dependsOn" in item["spec"]:
            item["spec"]["dependsOn"] = [
                dependency
                for dependency in item["spec"]["dependsOn"]
                if dependency["name"] not in names
            ]
        kept.append(item)
    return {"apiVersion": "v1", "kind": "List", "items": kept}


def main():
    try:
        result = without(read_objects(sys.stdin.read()), set(sys.argv[1:]))
    except KeyError as error:
        print(
            f"ERROR: --without names no Kustomization in the overlay: {error.args[0]}",
            file=sys.stderr,
        )
        return 2
    json.dump(result, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
