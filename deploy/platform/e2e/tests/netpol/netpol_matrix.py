"""Load and describe the NetworkPolicy probe matrix."""

from __future__ import annotations

import importlib.util
import ipaddress
from dataclasses import dataclass, field
from pathlib import Path
from types import ModuleType

REPO = Path(__file__).resolve().parents[5]
VERDICTS = ("allow", "deny")
DESTINATION_KINDS = ("service", "pod", "host")
NODE_HOST = "node"
MAX_PORT = 65535


def load_module(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


helm_yaml = load_module("helm_yaml", REPO / "deploy/platform/lint/helm_yaml.py")


class MatrixError(Exception):
    pass


@dataclass(frozen=True)
class Row:
    number: int
    namespace: str
    labels: dict[str, str]
    kind: str
    target: str
    port: int
    expect: str
    pod_namespace: str = ""
    pod_labels: dict[str, str] = field(default_factory=dict)

    @property
    def destination(self) -> str:
        if self.kind == "pod":
            return f"pod {self.pod_namespace}/{selector(self.pod_labels)}:{self.port}"
        return f"{self.target}:{self.port}"

    def describe(self) -> str:
        source = f"{self.namespace} {{{selector(self.labels)}}}"
        return f"#{self.number} {source} -> {self.destination} expect {self.expect}"


def selector(labels: dict[str, str]) -> str:
    return ",".join(f"{key}={value}" for key, value in sorted(labels.items()))


def parse(text: str) -> list[Row]:
    wrapped = "rows:\n" + "".join(f"  {line}\n" for line in text.splitlines())
    document = helm_yaml.parse_documents([wrapped])[0]
    items = document.get("rows")
    if not isinstance(items, list) or not items:
        raise MatrixError("the matrix is not a non-empty list of rows")
    return [row_from(number, item) for number, item in enumerate(items, start=1)]


def load(path: Path) -> list[Row]:
    return parse(path.read_text())


def row_from(number: int, item: object) -> Row:
    if not isinstance(item, dict):
        raise MatrixError(f"row #{number} is not a mapping")
    source = item.get("from") or {}
    destination = item.get("to") or {}
    expect = item.get("expect")
    if expect not in VERDICTS:
        raise MatrixError(f"row #{number}: expect must be allow or deny, not {expect!r}")
    namespace = source.get("namespace")
    if not isinstance(namespace, str) or not namespace:
        raise MatrixError(f"row #{number}: from.namespace is missing")
    labels = string_map(number, "from.labels", source.get("labels") or {})
    port = destination.get("port")
    if not isinstance(port, int) or not 0 < port <= MAX_PORT:
        raise MatrixError(f"row #{number}: to.port must be a TCP port, not {port!r}")
    kinds = [kind for kind in DESTINATION_KINDS if kind in destination]
    if len(kinds) != 1:
        raise MatrixError(f"row #{number}: to needs exactly one of {', '.join(DESTINATION_KINDS)}")
    kind = kinds[0]
    value = destination[kind]
    if kind == "pod":
        pod = value if isinstance(value, dict) else {}
        pod_namespace = pod.get("namespace")
        pod_labels = string_map(number, "to.pod.labels", pod.get("labels") or {})
        if not isinstance(pod_namespace, str) or not pod_labels:
            raise MatrixError(f"row #{number}: to.pod needs a namespace and labels")
        return Row(number, namespace, labels, kind, "", port, expect, pod_namespace, pod_labels)
    if kind == "service" and (not isinstance(value, str) or value.count(".") != 1):
        raise MatrixError(f"row #{number}: to.service must be <name>.<namespace>, not {value!r}")
    if kind == "host" and not valid_host(value):
        raise MatrixError(f"row #{number}: to.host must be an IPv4 address or node, not {value!r}")
    return Row(number, namespace, labels, kind, value, port, expect)


def string_map(number: int, name: str, value: object) -> dict[str, str]:
    if not isinstance(value, dict) or not all(
        isinstance(key, str) and isinstance(item, str) for key, item in value.items()
    ):
        raise MatrixError(f"row #{number}: {name} must map strings to strings")
    return dict(value)


def valid_host(value: object) -> bool:
    if value == NODE_HOST:
        return True
    if not isinstance(value, str):
        return False
    try:
        ipaddress.IPv4Address(value)
    except ValueError:
        return False
    return True
