from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEPENDENCY_TABLES = {"dependencies", "dev-dependencies", "build-dependencies"}
# No known semver incompatibilities require an exact third-party pin.
EXACT_PIN_ALLOWLIST: dict[tuple[str, str, str], str] = {}


def manifest_paths(root: Path) -> list[Path]:
    return [
        root / "rust/Cargo.toml",
        root / "cli/Cargo.toml",
        *sorted(root.glob("rust/crates/*/Cargo.toml")),
        *sorted(root.glob("templates/backend/**/Cargo.toml*")),
    ]


def parse_manifest(text: str) -> dict:
    text = re.sub(r"\{%.*?%\}", "", text, flags=re.DOTALL)
    text = text.replace("{{ context.baukit_dependencies }}", "")
    text = re.sub(r"\{\{.*?\}\}", "template_app", text, flags=re.DOTALL)
    return tomllib.loads(text)


def exact_pins(document: dict) -> set[tuple[str, str]]:
    pins = set()
    for table, entries in document.items():
        if not isinstance(entries, dict):
            continue
        if table not in DEPENDENCY_TABLES:
            pins.update(exact_pins(entries))
            continue
        for alias, requirement in entries.items():
            package = alias
            if isinstance(requirement, dict):
                package = requirement.get("package", alias)
                requirement = requirement.get("version", "")
            if package.startswith("baukit-"):
                continue
            if re.search(r"(?:^|,)\s*=(?!=)", requirement):
                pins.add((package, requirement))
    return pins


class RustDependencyRequirementsTest(unittest.TestCase):
    def test_repository_has_only_justified_exact_third_party_pins(self) -> None:
        found = set()
        for path in manifest_paths(ROOT):
            relative = path.relative_to(ROOT).as_posix()
            with self.subTest(manifest=relative):
                document = parse_manifest(path.read_text())
                found.update(
                    (relative, package, version)
                    for package, version in exact_pins(document)
                )
        self.assertEqual(
            found,
            set(EXACT_PIN_ALLOWLIST),
            "Exact pins need a documented semver incompatibility",
        )
        for pin, reason in EXACT_PIN_ALLOWLIST.items():
            with self.subTest(pin=pin):
                self.assertTrue(reason.strip(), "Exact pin exceptions need a reason")

    def test_detects_string_inline_and_multiline_dependency_pins(self) -> None:
        document = parse_manifest('''
            [workspace.dependencies]
            tokio = "=1.53.1"
            serde = { version = "=1.0.229", features = ["derive"] }
            [dev-dependencies.rustls]
            version = " =0.23.45"
            [target.'cfg(unix)'.build-dependencies]
            cc = " >=1.0, =1.2.0"
        ''')
        self.assertEqual(exact_pins(document), {
            ("tokio", "=1.53.1"), ("serde", "=1.0.229"),
            ("rustls", " =0.23.45"), ("cc", " >=1.0, =1.2.0"),
        })

    def test_caret_requirements_and_exact_baukit_dependencies_are_allowed(self) -> None:
        document = parse_manifest('''
            [package]
            version = "0.6.0"
            [dependencies]
            baukit-core = { version = "=0.6.0", path = "../baukit-core" }
            core = { package = "baukit-core", version = "=0.6.0" }
            tokio = "1.53.1"
            uuid = "^1.26.1"
            sqlx = "0.9.0"
            serde = { workspace = true }
        ''')
        self.assertEqual(exact_pins(document), set())

    def test_baukit_alias_does_not_hide_a_third_party_pin(self) -> None:
        document = parse_manifest('''
            [dependencies]
            baukit-tokio = { package = "tokio", version = "=1.53.1" }
        ''')
        self.assertEqual(exact_pins(document), {("tokio", "=1.53.1")})

    def test_templates_are_checked_inside_conditional_blocks(self) -> None:
        document = parse_manifest('''
            [workspace]
            members = ["crates/{{ context.app_name }}-domain"]
            [workspace.dependencies]
            {{ context.baukit_dependencies }}
            {% if context.worker %}tokio = { version = "=1.53.1" }
            {% endif %}
            {{ context.app_crate }}-domain = { path = "crates/{{ context.app_name }}-domain" }
        ''')
        self.assertEqual(exact_pins(document), {("tokio", "=1.53.1")})


if __name__ == "__main__":
    unittest.main()
