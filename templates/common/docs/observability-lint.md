# Observability metric-name lint

Dashboards, alerts, and recording rules reference metric names as strings.
Nothing in a compiler catches a renamed metric, so a panel keeps rendering an
empty graph and an alert keeps not firing. Baukit ships a linter that compares
every name referenced by an observability file against the set of names that are
supposed to exist.

The linter lives in Baukit, at
`deploy/observability/lint/check-metric-names.py`. It knows Baukit's own metric
vocabulary. It does not know the product's, and it does not know where the
product keeps its dashboards. A small shim supplies both and calls the linter.

## The shim

Write `scripts/observability-lint.py`. The `observability-lint` job in
`.github/workflows/ci.yml` looks for exactly that path: when the file is absent
the job reports that this product declares no dashboards and passes; when it is
present the job clones Baukit at the matching tag and runs the shim against the
linter it finds there.

```python
#!/usr/bin/env python3
"""Run Baukit's observability linter with this product's metric names."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

PRODUCT_METRICS = (
    "{{ context.app_crate }}_items_created_total",
    "{{ context.app_crate }}_items_request_duration_seconds",
)

PRODUCT_HISTOGRAMS = ("{{ context.app_crate }}_items_request_duration_seconds",)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} /path/to/check-metric-names.py", file=sys.stderr)
        return 2

    linter_path = Path(sys.argv[1]).resolve()
    spec = importlib.util.spec_from_file_location("baukit_observability_lint", linter_path)
    if spec is None or spec.loader is None:
        print(f"could not load the Baukit linter from {linter_path}", file=sys.stderr)
        return 2

    linter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(linter)

    root = Path(__file__).resolve().parents[1]
    linter.ROOT = root
    linter.OBSERVABILITY = root / "deploy" / "observability"
    linter.SPEC_METRICS += PRODUCT_METRICS
    for name in PRODUCT_HISTOGRAMS:
        linter.HISTOGRAM_METRICS.add(name)
    return linter.main()


if __name__ == "__main__":
    raise SystemExit(main())
```

## The contract

Four module globals and one function. These are what the shim is allowed to
touch, and what Baukit keeps stable across releases:

| Name | Type | Meaning |
|---|---|---|
| `linter.ROOT` | `Path` | Repository root the linter resolves paths against. |
| `linter.OBSERVABILITY` | `Path` | Directory holding dashboards, alerts, and recording rules. |
| `linter.SPEC_METRICS` | `tuple[str, ...]` | Every metric name that may be referenced. Extend with `+=`. |
| `linter.HISTOGRAM_METRICS` | `set[str]` | Names whose `_bucket`, `_sum`, and `_count` suffixes are also valid. Extend with `.add`. |
| `linter.main()` | `() -> int` | Runs the check and returns a process exit code. |

Extend the collections; never replace them. Assigning over `SPEC_METRICS` drops
Baukit's own names and turns every platform metric into a lint failure.

Run it locally the same way CI does:

```sh
git clone --branch v{{ context.template_version }} --depth 1 \
  https://github.com/PatrickKoss/baukit.git /tmp/baukit
python3 scripts/observability-lint.py /tmp/baukit/deploy/observability/lint/check-metric-names.py
```
