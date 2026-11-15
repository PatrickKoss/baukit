"""Record which pinned charts ship CRDs and Helm tests, and keep the CI policies in sync with it."""

from __future__ import annotations

import argparse
import fnmatch
import json
import re
import sys
import tarfile
import tomllib
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import chart_source
import helm_yaml

HERE = Path(__file__).resolve().parent
DATA_FILE = HERE / "chart-contents.toml"
POLICY_DIR = HERE / "policies"
POLICY_LISTS = {
    "crds": ("helmrelease-crd-policy.yaml", "chartsWithCrds"),
    "tests": ("helmrelease-helm-tests.yaml", "chartsWithTests"),
}
CRD_MEMBER = re.compile(r"^[^/]+(/charts/[^/]+)*/crds/[^/]+\.(ya?ml|json)$")
TEST_HOOK = re.compile(r"""^\s+["']?helm\.sh/hook["']?:\s*["']?[^\n]*\btest\b""", re.MULTILINE)
HEADER = (
    "# Written by chart_contents.py --write from the pinned charts and their renders.\n"
    "# crds: the chart or a subchart has a Helm crds/ directory.\n"
    "# tests: the rendered chart contains a helm.sh/hook: test object.\n"
)


class ContentsError(Exception):
    pass


@dataclass(frozen=True)
class Contents:
    crds: bool
    tests: bool


def chart_reference(release: dict, sources: dict | None = None) -> tuple[str, str]:
    try:
        return chart_source.release_chart(release, sources or {})
    except chart_source.ChartSourceError as error:
        raise ContentsError(str(error)) from error


def chart_key(name: str, version: str) -> str:
    return name if name.startswith("./") else f"{name}@{version}"


def archive_has_crds(archive: Path) -> bool:
    with tarfile.open(archive) as tar:
        return any(CRD_MEMBER.match(member) for member in tar.getnames())


def local_chart_has_crds(chart: Path) -> bool:
    return any(path.is_file() for path in chart.glob("**/crds/*.y*ml"))


def find_archive(chart_cache: Path, name: str, version: str) -> Path:
    matches = [
        archive
        for archive in sorted(chart_cache.glob("*.tgz"))
        if fnmatch.fnmatchcase(archive.name, f"{name}-{version}-*.tgz")
    ]
    if not matches:
        raise ContentsError(f"chart {name} {version} is not in {chart_cache}")
    return matches[0]


def releases_by_render(rendered: Path) -> list[tuple[dict, Path, dict]]:
    found = []
    for base in sorted(rendered.glob("base-*.yaml")):
        documents = [
            document
            for document in helm_yaml.split_documents(base.read_text())
            if re.search(r"^kind: (HelmRelease|OCIRepository)$", document, re.MULTILINE)
        ]
        base_name = base.stem.removeprefix("base-")
        objects = helm_yaml.parse_documents(documents)
        sources = chart_source.oci_sources(objects)
        for release in objects:
            if release.get("kind") != "HelmRelease":
                continue
            name = release["metadata"]["name"]
            found.append((release, rendered / f"chart-{base_name}-{name}.yaml", sources))
    return found


def collect(rendered: Path, chart_cache: Path, repo_root: Path) -> dict[str, Contents]:
    contents: dict[str, Contents] = {}
    for release, chart_render, sources in releases_by_render(rendered):
        name, version = chart_reference(release, sources)
        if name.startswith("./"):
            crds = local_chart_has_crds(repo_root / name)
        else:
            crds = archive_has_crds(find_archive(chart_cache, name, version))
        if not chart_render.is_file():
            raise ContentsError(f"no chart render {chart_render.name}")
        tests = bool(TEST_HOOK.search(chart_render.read_text()))
        contents[chart_key(name, version)] = Contents(crds, tests)
    return contents


def render_data(contents: dict[str, Contents]) -> str:
    sections = [
        f'["{key}"]\ncrds = {str(item.crds).lower()}\ntests = {str(item.tests).lower()}\n'
        for key, item in sorted(contents.items())
    ]
    return HEADER + "\n" + "\n".join(sections)


def load_data(path: Path) -> dict[str, Contents]:
    raw = tomllib.loads(path.read_text())
    return {key: Contents(bool(item["crds"]), bool(item["tests"])) for key, item in raw.items()}


def chart_names(contents: dict[str, Contents], field: str) -> list[str]:
    return sorted({key.split("@")[0] for key, item in contents.items() if getattr(item, field)})


def variable_pattern(variable: str) -> re.Pattern[str]:
    return re.compile(rf"(- name: {variable}\n\s+expression: ')(\[.*?\])(')")


def policy_list(policy: Path, variable: str) -> list[str]:
    match = variable_pattern(variable).search(policy.read_text())
    if not match:
        raise ContentsError(f"{policy.name} has no {variable} variable")
    return json.loads(match.group(2))


def write_policy_list(policy: Path, variable: str, names: list[str]) -> None:
    text = policy.read_text()
    replacement = json.dumps(names)
    updated, count = variable_pattern(variable).subn(
        lambda m: m.group(1) + replacement + m.group(3), text
    )
    if count != 1:
        raise ContentsError(f"{policy.name} has no {variable} variable")
    policy.write_text(updated)


def check(contents: dict[str, Contents], data_file: Path, policy_dir: Path) -> list[str]:
    problems = []
    recorded = load_data(data_file) if data_file.is_file() else {}
    for key in sorted(set(contents) | set(recorded)):
        if contents.get(key) != recorded.get(key):
            problems.append(
                f"{data_file.name}: {key} is {recorded.get(key)}, "
                f"the pinned chart has {contents.get(key)}"
            )
    for field, (policy_name, variable) in POLICY_LISTS.items():
        expected = chart_names(recorded, field)
        actual = policy_list(policy_dir / policy_name, variable)
        if actual != expected:
            problems.append(
                f"{policy_name}: {variable} is {actual}, {data_file.name} says {expected}"
            )
    return problems


def write(contents: dict[str, Contents], data_file: Path, policy_dir: Path) -> None:
    data_file.write_text(render_data(contents))
    for field, (policy_name, variable) in POLICY_LISTS.items():
        write_policy_list(policy_dir / policy_name, variable, chart_names(contents, field))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--chart-cache", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=HERE.parents[3])
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args(argv)
    try:
        contents = collect(args.rendered, args.chart_cache, args.repo_root)
        if args.write:
            write(contents, DATA_FILE, POLICY_DIR)
            return 0
        problems = check(contents, DATA_FILE, POLICY_DIR)
    except (ContentsError, helm_yaml.YamlError, OSError, KeyError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    for problem in problems:
        print(f"ERROR: {problem}", file=sys.stderr)
    if problems:
        print("Run chart_contents.py --write and review the diff.", file=sys.stderr)
        return 1
    print(f"Chart contents: {len(contents)} charts match {DATA_FILE.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
