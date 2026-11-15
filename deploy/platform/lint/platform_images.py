"""Scan every container image in the rendered platform and every tracked Dockerfile with trivy."""

from __future__ import annotations

import argparse
import base64
import binascii
import datetime
import itertools
import json
import re
import subprocess
import sys
import textwrap
import tomllib
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

import helm_yaml

process_runner = helm_yaml.process_runner

IMAGE_KEYS = frozenset({"image", "imageName"})
IMAGE_DATA_KEY = re.compile(r"(?:^|_)IMAGE$")
IMAGE_REFERENCE = re.compile(
    r"^[a-z0-9]+(?:[._-][a-z0-9]+)*(?::[0-9]+)?"
    r"(?:/[a-z0-9]+(?:(?:[._]|__|-+)[a-z0-9]+)*)*"
    r"(?::[A-Za-z0-9_][A-Za-z0-9._-]{0,127})?"
    r"(?:@sha256:[a-f0-9]{64})?$"
)
FOLLOW_UP_TASK = re.compile(r"\b[A-Z]{1,3}[0-9]{1,2}\b")
ALLOWLIST_SECTIONS = ("images", "dockerfiles")
INLINE_PATCH_IMAGE = re.compile(r"path:\s*\S*/image\s*\n\s*value:\s*[\"']?([^\s\"']+)")
JINJA_STATEMENT = re.compile(r"\{%-?.*?-?%\}", re.DOTALL)
JINJA_EXPRESSION = re.compile(r"\{\{-?.*?-?\}\}", re.DOTALL)
JINJA_PLACEHOLDER = "placeholder"
SEVERITIES = "HIGH,CRITICAL"
SCAN_TIMEOUT = "20m"
RAW_ROOTS = ("deploy/platform",)
DOCKERFILE_ROOTS = ("deploy", "templates", "examples")
RAW_EXCLUDED = ("tests", "fixtures")


class ScanError(Exception):
    pass


def patched_image(value: dict) -> str | None:
    path, image = value.get("path"), value.get("value")
    if isinstance(path, str) and path.endswith("/image") and isinstance(image, str):
        return image
    return None


def secret_images(value: dict) -> Iterable[str]:
    """Image references a Secret carries under *_IMAGE keys, such as the Barman plugin sidecar."""
    if value.get("kind") != "Secret":
        return
    for key, encoded in (value.get("data") or {}).items():
        if IMAGE_DATA_KEY.search(str(key)) and isinstance(encoded, str):
            try:
                yield base64.b64decode(encoded, validate=True).decode().strip()
            except (binascii.Error, UnicodeDecodeError) as error:
                raise ScanError(f"Secret data {key} is not base64 text: {error}") from error
    for key, plain in (value.get("stringData") or {}).items():
        if IMAGE_DATA_KEY.search(str(key)) and isinstance(plain, str):
            yield plain.strip()


def images_in(value: object) -> Iterable[str]:
    if isinstance(value, dict):
        yield from secret_images(value)
        patched = patched_image(value)
        if patched is not None:
            yield patched
        for key, child in value.items():
            if key in IMAGE_KEYS and isinstance(child, str):
                yield child
            elif key == "patch" and isinstance(child, str):
                yield from INLINE_PATCH_IMAGE.findall(child)
            else:
                yield from images_in(child)
    elif isinstance(value, list):
        for child in value:
            yield from images_in(child)


def is_image_reference(candidate: str) -> bool:
    return bool(IMAGE_REFERENCE.match(candidate)) and (":" in candidate or "/" in candidate)


def extract_images(objects: Iterable[object]) -> list[str]:
    found = {image for obj in objects for image in images_in(obj) if is_image_reference(image)}
    return sorted(found)


def load_documents(paths: Iterable[Path]) -> list[object]:
    """Parse every document, wrapped under one key so top-level lists (patch files) parse too."""
    documents = [
        "root:\n" + textwrap.indent(document, "  ")
        for path in paths
        for document in helm_yaml.split_documents(path.read_text())
        if document.strip()
    ]
    return [parsed.get("root") for parsed in helm_yaml.parse_documents(documents)]


def raw_manifests(repo_root: Path, roots: Sequence[str], excluded: Sequence[str]) -> list[Path]:
    result = process_runner.run_process(
        ["git", "ls-files", "--", *(f"{root}/*.yaml" for root in roots)],
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    if result.returncode != 0:
        raise ScanError(f"git ls-files failed: {result.stderr.strip()}")
    return [
        repo_root / line
        for line in result.stdout.splitlines()
        if line and not any(f"/{part}/" in f"/{line}" for part in excluded)
    ]


def load_allowlist(path: Path) -> dict[str, dict[str, Allowance]]:
    data = tomllib.loads(path.read_text())
    unknown = set(data) - set(ALLOWLIST_SECTIONS)
    if unknown:
        raise ScanError(f"{path}: unknown sections {sorted(unknown)}")
    return {
        section: {key: Allowance.parse(key, entry) for key, entry in data.get(section, {}).items()}
        for section in ALLOWLIST_SECTIONS
    }


@dataclass(frozen=True)
class Allowance:
    expires: datetime.date | None
    reason: str
    ids: frozenset[str]

    @classmethod
    def parse(cls, key: str, entry: object) -> Allowance:
        if not isinstance(entry, dict):
            raise ScanError(f"{key}: entry must be a table")
        expires, reason, ids = entry.get("expires"), entry.get("reason", ""), entry.get("ids", [])
        if not isinstance(ids, list) or not all(isinstance(item, str) for item in ids):
            raise ScanError(f"{key}: ids must be a list of strings")
        return cls(
            expires if isinstance(expires, datetime.date) else None,
            reason if isinstance(reason, str) else "",
            frozenset(ids),
        )


def allowance_problems(key: str, allowance: Allowance, today: datetime.date) -> list[str]:
    problems = []
    if not FOLLOW_UP_TASK.search(allowance.reason):
        problems.append(f"{key}: reason must name the follow-up task")
    if allowance.expires is None:
        problems.append(f"{key}: expires must be a YYYY-MM-DD date")
    elif allowance.expires < today:
        problems.append(f"{key}: expired on {allowance.expires}")
    if not allowance.ids:
        problems.append(f"{key}: ids is empty; remove the entry")
    return problems


def compare(
    reported: dict[str, set[str]], allowed: dict[str, Allowance], today: datetime.date
) -> list[str]:
    """Fail on unlisted findings, stale listed IDs and listed targets that are gone."""
    problems = []
    for key, allowance in sorted(allowed.items()):
        problems.extend(allowance_problems(key, allowance, today))
        if key not in reported:
            problems.append(f"{key}: no findings left; remove the entry")
            continue
        for stale in sorted(allowance.ids - reported[key]):
            problems.append(f"{key}: {stale} is no longer reported; remove it from the list")
    for key, found in sorted(reported.items()):
        listed = allowed[key].ids if key in allowed else frozenset()
        for new in sorted(found - listed):
            problems.append(f"{key}: {new} is not in the allowlist")
    return problems


def render_jinja_superset(text: str) -> str:
    """Drop Jinja statements and give each expression its own name, so every branch is scanned."""
    counter = itertools.count(1)
    without_statements = JINJA_STATEMENT.sub("", text)
    return JINJA_EXPRESSION.sub(lambda _: f"{JINJA_PLACEHOLDER}{next(counter)}", without_statements)


def tracked_dockerfiles(repo_root: Path) -> list[Path]:
    result = process_runner.run_process(
        ["git", "ls-files", "--", *(f"{root}/*Dockerfile*" for root in DOCKERFILE_ROOTS)],
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    if result.returncode != 0:
        raise ScanError(f"git ls-files failed: {result.stderr.strip()}")
    return [repo_root / line for line in result.stdout.splitlines() if line]


def stage_dockerfiles(dockerfiles: Sequence[Path], repo_root: Path, stage: Path) -> dict[str, Path]:
    """Copy each Dockerfile to its own directory; map its repository path to the staged file."""
    staged = {}
    for source in dockerfiles:
        name = source.relative_to(repo_root).as_posix().replace("/", "__")
        text = source.read_text()
        if source.suffix == ".jinja":
            name = name.removesuffix(".jinja")
            text = render_jinja_superset(text)
        target = stage / name.replace("Dockerfile", "") / "Dockerfile"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        staged[source.relative_to(repo_root).as_posix()] = target
    return staged


Runner = Callable[[list[str]], tuple[int, str]]


def run_command(command: list[str]) -> tuple[int, str]:
    print("+ " + " ".join(command), flush=True)
    result = process_runner.run_process(command, stdout=subprocess.PIPE, text=True)
    return result.returncode, result.stdout


def trivy_json(command: list[str], run: Runner) -> dict:
    code, output = run(command)
    if code != 0:
        raise ScanError(f"trivy failed with exit code {code}: {' '.join(command)}")
    try:
        report = json.loads(output)
    except json.JSONDecodeError as error:
        raise ScanError(f"trivy printed no JSON report: {error}") from error
    if not isinstance(report, dict):
        raise ScanError("trivy printed a JSON report that is not an object")
    return report


def report_ids(report: dict, field: str, id_key: str) -> dict[str, set[str]]:
    """Map each target to its finding IDs. Misconfigurations count only when they FAIL."""
    found: dict[str, set[str]] = {}
    for result in report.get("Results") or []:
        target = str(result.get("Target", ""))
        for item in result.get(field) or []:
            if field == "Misconfigurations" and item.get("Status") != "FAIL":
                continue
            found.setdefault(target, set()).add(str(item[id_key]))
    return found


def scan_images(
    images: Sequence[str], cache: Path, run: Runner, config: Path | None = None
) -> dict[str, set[str]]:
    common = ["--cache-dir", str(cache), "--quiet"]
    if config is not None:
        common += ["--config", str(config)]
    for database in ("--download-db-only", "--download-java-db-only"):
        if run(["trivy", "image", *common, database])[0] != 0:
            raise ScanError(f"trivy {database} failed")
    reported = {}
    for image in images:
        command = [
            "trivy", "image", *common,
            "--skip-db-update", "--skip-java-db-update",
            "--scanners", "vuln", "--timeout", SCAN_TIMEOUT,
            "--severity", SEVERITIES, "--ignore-unfixed", "--format", "json", image,
        ]  # fmt: skip
        per_target = report_ids(trivy_json(command, run), "Vulnerabilities", "VulnerabilityID")
        reported[image] = set().union(*per_target.values()) if per_target else set()
    return reported


def scan_dockerfiles(
    staged: dict[str, Path], stage: Path, cache: Path, run: Runner
) -> dict[str, set[str]]:
    command = [
        "trivy", "config", "--cache-dir", str(cache), "--quiet",
        "--severity", SEVERITIES, "--format", "json", str(stage),
    ]  # fmt: skip
    per_target = report_ids(trivy_json(command, run), "Misconfigurations", "ID")
    by_staged = {path.relative_to(stage).as_posix(): source for source, path in staged.items()}
    reported = {source: set() for source in staged}
    for target, ids in per_target.items():
        source = by_staged.get(target)
        if source is None:
            raise ScanError(f"trivy reported {target}, which maps to no tracked Dockerfile")
        reported[source] |= ids
    return reported


def drop_clean(reported: dict[str, set[str]]) -> dict[str, set[str]]:
    return {key: ids for key, ids in reported.items() if ids}


def main(argv: list[str] | None = None, run: Runner = run_command) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--allowlist", type=Path, required=True)
    parser.add_argument(
        "--trivy-config", type=Path, help="trivy.yaml, for example registry mirrors"
    )
    parser.add_argument("--today", type=datetime.date.fromisoformat, default=datetime.date.today())
    args = parser.parse_args(argv)

    try:
        allowlist = load_allowlist(args.allowlist)
        manifests = sorted(args.rendered.glob("*.yaml"))
        manifests += raw_manifests(args.repo_root, RAW_ROOTS, RAW_EXCLUDED)
        images = extract_images(load_documents(manifests))
        if not images:
            raise ScanError(f"no container images found under {args.rendered}")
        print(f"==> {len(images)} images", flush=True)
        image_findings = scan_images(images, args.cache, run, args.trivy_config)
        stage = args.work / "dockerfiles"
        staged = stage_dockerfiles(tracked_dockerfiles(args.repo_root), args.repo_root, stage)
        dockerfile_findings = scan_dockerfiles(staged, stage, args.cache, run)
    except (ScanError, helm_yaml.YamlError, OSError, tomllib.TOMLDecodeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    problems = compare(drop_clean(image_findings), allowlist["images"], args.today)
    problems += compare(drop_clean(dockerfile_findings), allowlist["dockerfiles"], args.today)
    for problem in problems:
        print(f"ERROR: {args.allowlist.name}: {problem}", file=sys.stderr)
    print(f"==> {len(problems)} allowlist problems", flush=True)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
