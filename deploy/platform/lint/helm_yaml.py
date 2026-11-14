"""Parse YAML documents with helm's fromYaml, since the Python stdlib has no YAML parser."""

from __future__ import annotations

import importlib.util
import json
import re
import tempfile
from collections.abc import Sequence
from pathlib import Path
from types import ModuleType

PROCESS_RUNNER = Path(__file__).resolve().parents[3] / "scripts" / "process_runner.py"
DOCUMENT_SEPARATOR = re.compile(r"^---[ \t]*$", re.MULTILINE)
MARKER = "# helm-yaml "
PARSER_CHART = "apiVersion: v2\nname: yaml-json\nversion: 0.0.0\n"
PARSER_TEMPLATE = """\
{{- range $path, $_ := .Files.Glob "files/*.yaml" }}
{{- $obj := fromYaml ($.Files.Get $path) }}
{{- if hasKey $obj "Error" }}{{ fail (printf "%s: %s" $path $obj.Error) }}{{ end }}
---
# helm-yaml {{ $path }}
{{ toJson $obj }}
{{- end }}
"""


class YamlError(Exception):
    pass


def load_module(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


process_runner = load_module("process_runner", PROCESS_RUNNER)


def split_documents(text: str) -> list[str]:
    return DOCUMENT_SEPARATOR.split(text)


def parse_documents(documents: Sequence[str]) -> list[dict]:
    """Return one mapping per document, in order. An empty document parses to {}."""
    with tempfile.TemporaryDirectory() as chart_dir:
        chart = Path(chart_dir)
        (chart / "templates").mkdir()
        (chart / "files").mkdir()
        (chart / "Chart.yaml").write_text(PARSER_CHART)
        (chart / "templates" / "parse.yaml").write_text(PARSER_TEMPLATE)
        for index, document in enumerate(documents):
            (chart / "files" / f"{index:06d}.yaml").write_text(document)
        result = process_runner.run_process(
            ["helm", "template", "yaml-json", str(chart)], capture_output=True, text=True
        )
    if result.returncode != 0:
        raise YamlError(f"helm could not parse the YAML:\n{result.stderr.strip()}")
    parsed: dict[int, dict] = {}
    index = -1
    for line in result.stdout.splitlines():
        if line.startswith(MARKER):
            index = int(Path(line.removeprefix(MARKER)).stem)
        elif line.startswith("{") and index >= 0:
            parsed[index] = json.loads(line)
    return [parsed.get(index, {}) for index in range(len(documents))]
