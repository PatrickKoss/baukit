"""Parse YAML documents with helm's fromYaml, since the Python stdlib has no YAML parser."""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import tempfile
from collections.abc import Sequence
from pathlib import Path

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


def split_documents(text: str) -> list[str]:
    return DOCUMENT_SEPARATOR.split(text)


def parse_documents(documents: Sequence[str]) -> list[dict]:
    """Return one mapping per document, in order. An empty document parses to {}."""
    helm = shutil.which("helm")
    if helm is None:
        raise YamlError("helm is required to parse YAML")
    with tempfile.TemporaryDirectory() as chart_dir:
        chart = Path(chart_dir)
        (chart / "templates").mkdir()
        (chart / "files").mkdir()
        (chart / "Chart.yaml").write_text(PARSER_CHART)
        (chart / "templates" / "parse.yaml").write_text(PARSER_TEMPLATE)
        for index, document in enumerate(documents):
            (chart / "files" / f"{index:06d}.yaml").write_text(document)
        result = subprocess.run(  # noqa: S603 -- argv is the helm found by shutil.which plus fixed flags, no shell
            [helm, "template", "yaml-json", str(chart)],
            capture_output=True,
            text=True,
            check=False,
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
