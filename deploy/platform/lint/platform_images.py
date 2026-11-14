"""Scan every container image in the rendered platform and every tracked Dockerfile with trivy."""

from __future__ import annotations

import argparse
import datetime
import re
import sys
import textwrap
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

import helm_yaml

process_runner = helm_yaml.process_runner

IMAGE_KEYS = frozenset({"image", "imageName"})
IMAGE_REFERENCE = re.compile(
    r"^[a-z0-9]+(?:[._-][a-z0-9]+)*(?::[0-9]+)?"
    r"(?:/[a-z0-9]+(?:(?:[._]|__|-+)[a-z0-9]+)*)*"
    r"(?::[A-Za-z0-9_][A-Za-z0-9._-]{0,127})?"
    r"(?:@sha256:[a-f0-9]{64})?$"
)
EXPIRY = re.compile(r"(?:^|\s)exp:(\d{4}-\d{2}-\d{2})(?:\s|$)")
INLINE_PATCH_IMAGE = re.compile(r"path:\s*\S*/image\s*\n\s*value:\s*[\"']?([^\s\"']+)")
JINJA_STATEMENT = re.compile(r"\{%-?.*?-?%\}", re.DOTALL)
JINJA_EXPRESSION = re.compile(r"\{\{-?.*?-?\}\}", re.DOTALL)
JINJA_PLACEHOLDER = "placeholder"
SEVERITIES = "HIGH,CRITICAL"
RAW_ROOTS = ("deploy/platform",)
RAW_EXCLUDED = ("tests", "fixtures")


class ScanError(Exception):
    pass


@dataclass(frozen=True)
class IgnoreProblem:
    line: int
    message: str

    def __str__(self) -> str:
        return f"line {self.line}: {self.message}"


def patched_image(value: dict) -> str | None:
    path, image = value.get("path"), value.get("value")
    if isinstance(path, str) and path.endswith("/image") and isinstance(image, str):
        return image
    return None


def images_in(value: object) -> Iterable[str]:
    if isinstance(value, dict):
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


def check_ignore_file(text: str, today: datetime.date) -> list[IgnoreProblem]:
    problems: list[IgnoreProblem] = []
    reason = ""
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line:
            reason = ""
            continue
        if line.startswith("#"):
            reason = f"{reason} {line.lstrip('#').strip()}".strip()
            continue
        problems.extend(entry_problems(number, line, reason, today))
        reason = ""
    return problems


def entry_problems(
    number: int, line: str, reason: str, today: datetime.date
) -> list[IgnoreProblem]:
    problems = []
    if not reason:
        problems.append(
            IgnoreProblem(number, f"{line.split(maxsplit=1)[0]} has no reason comment above it")
        )
    expiry = EXPIRY.search(line)
    if expiry is None:
        problems.append(
            IgnoreProblem(number, f"{line.split(maxsplit=1)[0]} has no exp:YYYY-MM-DD expiry")
        )
        return problems
    try:
        expires = datetime.date.fromisoformat(expiry.group(1))
    except ValueError:
        problems.append(IgnoreProblem(number, f"{expiry.group(1)} is not a valid date"))
        return problems
    if expires < today:
        problems.append(IgnoreProblem(number, f"{line.split(maxsplit=1)[0]} expired on {expires}"))
    return problems


def render_jinja_superset(text: str) -> str:
    """Drop Jinja statements and fill expressions, so every branch of the template is scanned."""
    return JINJA_EXPRESSION.sub(JINJA_PLACEHOLDER, JINJA_STATEMENT.sub("", text))


def tracked_dockerfiles(repo_root: Path) -> list[Path]:
    result = process_runner.run_process(
        ["git", "ls-files", "--", "*Dockerfile*"],
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    if result.returncode != 0:
        raise ScanError(f"git ls-files failed: {result.stderr.strip()}")
    return [repo_root / line for line in result.stdout.splitlines() if line]


def stage_dockerfiles(dockerfiles: Sequence[Path], repo_root: Path, stage: Path) -> list[Path]:
    staged = []
    for source in dockerfiles:
        name = source.relative_to(repo_root).as_posix().replace("/", "__")
        text = source.read_text()
        if source.suffix == ".jinja":
            name = name.removesuffix(".jinja")
            text = render_jinja_superset(text)
        target = stage / name.replace("Dockerfile", "") / "Dockerfile"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        staged.append(target)
    return staged


Runner = Callable[[list[str]], int]


def run_command(command: list[str]) -> int:
    print("+ " + " ".join(command), flush=True)
    return process_runner.run_process(command).returncode


def scan_images(images: Sequence[str], cache: Path, ignore_file: Path, run: Runner) -> list[str]:
    common = ["--cache-dir", str(cache), "--quiet"]
    if run(["trivy", "image", *common, "--download-db-only"]) != 0:
        raise ScanError("trivy could not download its vulnerability database")
    if run(["trivy", "image", *common, "--download-java-db-only"]) != 0:
        raise ScanError("trivy could not download its Java database")
    failed = []
    for image in images:
        command = [
            "trivy", "image", *common,
            "--skip-db-update", "--skip-java-db-update",
            "--severity", SEVERITIES, "--ignore-unfixed", "--exit-code", "1",
            "--ignorefile", str(ignore_file), image,
        ]  # fmt: skip
        if run(command) != 0:
            failed.append(image)
    return failed


def scan_configs(stage: Path, cache: Path, ignore_file: Path, run: Runner) -> bool:
    command = [
        "trivy", "config", "--cache-dir", str(cache), "--quiet",
        "--severity", SEVERITIES, "--exit-code", "1",
        "--ignorefile", str(ignore_file), str(stage),
    ]  # fmt: skip
    return run(command) == 0


def main(argv: list[str] | None = None, run: Runner = run_command) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--ignore-file", type=Path, required=True)
    parser.add_argument("--today", type=datetime.date.fromisoformat, default=datetime.date.today())
    args = parser.parse_args(argv)

    problems = check_ignore_file(args.ignore_file.read_text(), args.today)
    for problem in problems:
        print(f"ERROR: {args.ignore_file}: {problem}", file=sys.stderr)
    try:
        manifests = sorted(args.rendered.glob("*.yaml"))
        manifests += raw_manifests(args.repo_root, RAW_ROOTS, RAW_EXCLUDED)
        images = extract_images(load_documents(manifests))
        if not images:
            raise ScanError(f"no container images found under {args.rendered}")
        print(f"==> {len(images)} images", flush=True)
        failed = scan_images(images, args.cache, args.ignore_file, run)
        stage = args.work / "dockerfiles"
        stage_dockerfiles(tracked_dockerfiles(args.repo_root), args.repo_root, stage)
        configs_clean = scan_configs(stage, args.cache, args.ignore_file, run)
    except (ScanError, helm_yaml.YamlError, OSError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for image in failed:
        print(f"ERROR: {image} has HIGH or CRITICAL findings with a fix", file=sys.stderr)
    if not configs_clean:
        print("ERROR: trivy config reported HIGH or CRITICAL Dockerfile findings", file=sys.stderr)
    return 1 if problems or failed or not configs_clean else 0


if __name__ == "__main__":
    sys.exit(main())
