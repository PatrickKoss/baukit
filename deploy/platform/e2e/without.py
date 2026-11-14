"""Drops the named Flux Kustomizations from a JSON List and from every dependsOn list.

Usage: without.py NAME... < list.json > list.json. Exits 2 when a NAME is not in the List.
"""

import json
import sys

KUSTOMIZATION_KIND = "Kustomization"


def is_kustomization(item):
    return item.get("kind") == KUSTOMIZATION_KIND and item.get("apiVersion", "").startswith(
        "kustomize.toolkit.fluxcd.io/"
    )


def without(document, names):
    present = {item["metadata"]["name"] for item in document["items"] if is_kustomization(item)}
    unknown = sorted(set(names) - present)
    if unknown:
        raise KeyError(", ".join(unknown))
    kept = []
    for item in document["items"]:
        if is_kustomization(item) and item["metadata"]["name"] in names:
            continue
        if is_kustomization(item) and "dependsOn" in item["spec"]:
            item["spec"]["dependsOn"] = [
                dependency
                for dependency in item["spec"]["dependsOn"]
                if dependency["name"] not in names
            ]
        kept.append(item)
    return {**document, "items": kept}


def main():
    try:
        result = without(json.load(sys.stdin), set(sys.argv[1:]))
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
