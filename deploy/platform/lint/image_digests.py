"""V5 digest policy: every rendered container image carries an @sha256 digest."""

from __future__ import annotations

import tomllib
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

import platform_images

DIGEST_MARK = "@sha256:"
SKIPPED_KINDS = frozenset({"HelmRelease"})


class ExemptionError(Exception):
    pass


@dataclass(frozen=True)
class Exemptions:
    rendered: Mapping[str, str]
    embedded: Mapping[str, str]


def load_exemptions(path: Path) -> Exemptions:
    data = tomllib.loads(path.read_text())
    unknown = set(data) - {"images", "embedded"}
    if unknown:
        raise ExemptionError(f"{path.name}: unknown sections {sorted(unknown)}")
    rendered = {key: _reason(key, entry) for key, entry in (data.get("images") or {}).items()}
    embedded = {}
    for key, entry in (data.get("embedded") or {}).items():
        _reason(key, entry)
        carrier = entry.get("carrier")
        if not isinstance(carrier, str) or not carrier:
            raise ExemptionError(f"{path.name}: embedded {key} needs a carrier image repository")
        embedded[key] = carrier
    return Exemptions(rendered, embedded)


def _reason(key: str, entry: object) -> str:
    if not isinstance(entry, dict) or not str(entry.get("reason", "")).strip():
        raise ExemptionError(f"{key}: exemption needs a reason")
    return str(entry["reason"])


def object_images(obj: object) -> list[str]:
    if isinstance(obj, dict) and obj.get("kind") in SKIPPED_KINDS:
        return []
    return platform_images.extract_images([obj])


def repository(image: str) -> str:
    name = image.split("@", 1)[0]
    last = name.rsplit("/", 1)[-1]
    return name.rsplit(":", 1)[0] if ":" in last else name


def problems(
    sources: Mapping[str, Iterable[tuple[str, object]]], exemptions: Exemptions
) -> list[str]:
    """sources maps a universe name to (object reference, object) pairs."""
    found: list[str] = []
    seen: set[str] = set()
    for universe, objects in sources.items():
        for ref, obj in objects:
            try:
                images = object_images(obj)
            except platform_images.ScanError as error:
                found.append(f"{universe}: {ref}: {error}")
                continue
            for image in images:
                seen.add(image)
                if DIGEST_MARK not in image and image not in exemptions.rendered:
                    found.append(f"{universe}: {ref} runs {image} without an @sha256 digest")
    for image in sorted(set(exemptions.rendered) - seen):
        found.append(f"exemption {image} matches no rendered image; remove the entry")
    pinned_repositories = {repository(image) for image in seen if DIGEST_MARK in image}
    for image, carrier in sorted(exemptions.embedded.items()):
        if carrier not in pinned_repositories:
            found.append(
                f"embedded {image} names carrier {carrier}, which no render pins by digest"
            )
    return found
