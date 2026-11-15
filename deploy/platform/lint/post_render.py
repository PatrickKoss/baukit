"""Apply a HelmRelease's kustomize post-renderers to its `helm template` output, as Flux does."""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path

import helm_yaml

SUPPORTED_KEYS = {"patches", "images"}
RENDERED_FILE = "rendered.yaml"


class PostRenderError(Exception):
    pass


def kustomizations(release: dict) -> list[dict]:
    steps = []
    for index, renderer in enumerate((release.get("spec") or {}).get("postRenderers") or []):
        kustomize = renderer.get("kustomize") if isinstance(renderer, dict) else None
        if not isinstance(kustomize, dict) or set(renderer) != {"kustomize"}:
            raise PostRenderError(f"postRenderers[{index}] is not a kustomize post-renderer")
        unsupported = sorted(set(kustomize) - SUPPORTED_KEYS)
        if unsupported:
            raise PostRenderError(f"postRenderers[{index}] uses unsupported keys {unsupported}")
        steps.append({"resources": [RENDERED_FILE], **kustomize})
    return steps


def apply(rendered: str, steps: list[dict], kustomize: str) -> str:
    for step in steps:
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / RENDERED_FILE).write_text(rendered)
            (work / "kustomization.yaml").write_text(json.dumps(step))
            result = helm_yaml.process_runner.run_process(
                [kustomize, "build", str(work)], capture_output=True, text=True
            )
        if result.returncode != 0:
            raise PostRenderError(f"kustomize post-render failed:\n{result.stderr.strip()}")
        rendered = result.stdout
    return rendered


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--kustomize", default="kustomize")
    args = parser.parse_args(argv)
    try:
        (release,) = helm_yaml.parse_documents([args.release.read_text()])
        steps = kustomizations(release)
        if steps:
            args.rendered.write_text(apply(args.rendered.read_text(), steps, args.kustomize))
    except (PostRenderError, helm_yaml.YamlError, OSError) as error:
        print(f"ERROR: {args.release}: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
