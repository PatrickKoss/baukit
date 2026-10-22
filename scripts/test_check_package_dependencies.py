from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "check_package_dependencies", Path(__file__).with_name("check-package-dependencies.py")
)
assert SPEC is not None and SPEC.loader is not None
check = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check)


class PackedDependenciesTest(unittest.TestCase):
    def test_local_specifiers_are_rejected_in_every_published_group(self) -> None:
        for group in check.DEPENDENCY_GROUPS:
            for protocol in check.LOCAL_PROTOCOLS:
                with self.subTest(group=group, protocol=protocol):
                    with tempfile.TemporaryDirectory() as temporary:
                        directory = Path(temporary)
                        (directory / "package.json").write_text(json.dumps({
                            "name": "@baukit/pack-test", "version": "1.0.0",
                            group: {"@baukit/sibling": protocol + "*"},
                        }))
                        with self.assertRaisesRegex(ValueError, f"{group}.*{protocol}"):
                            check.check_package(directory)

    def test_pack_output_of_npm_11_and_npm_12_is_read(self) -> None:
        entry = {"name": "@baukit/pack-test", "filename": "baukit-pack-test-1.0.0.tgz"}
        for output in ([entry], {"@baukit/pack-test": entry}):
            with self.subTest(shape=type(output).__name__):
                self.assertEqual(
                    check.packed_filename(json.dumps(output), Path(".")),
                    "baukit-pack-test-1.0.0.tgz",
                )
        with self.assertRaisesRegex(ValueError, "expected one npm archive"):
            check.packed_filename(json.dumps([entry, entry]), Path("."))

    def test_registry_ranges_and_local_dev_dependencies_are_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "package.json").write_text(json.dumps({
                "name": "@baukit/pack-test", "version": "1.0.0",
                **{group: {"@baukit/sibling": "^1.0.0"} for group in check.DEPENDENCY_GROUPS},
                "devDependencies": {"@baukit/sibling": "workspace:*"},
            }))
            result = subprocess.run(
                [sys.executable, str(Path(check.__file__)), str(directory)],
                capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("Checked npm dependency specifiers in 1 packed packages", result.stdout)

    def test_current_packages_pack_with_installable_dependencies(self) -> None:
        manifests = check.typescript_manifests(check.ROOT)
        self.assertIn("@baukit/suite-client", manifests)
        result = subprocess.run(
            [sys.executable, str(Path(check.__file__))], capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"in {len(manifests)} packed packages", result.stdout)

    def test_command_fails_for_a_workspace_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "package.json").write_text(json.dumps({
                "name": "@baukit/pack-test", "version": "1.0.0",
                "dependencies": {"@baukit/sibling": "workspace:*"},
            }))
            result = subprocess.run(
                [sys.executable, str(Path(check.__file__)), str(directory)],
                capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("dependencies: @baukit/sibling requires workspace:*", result.stderr)

    def test_suite_client_installs_with_its_peers_from_npm_archives(self) -> None:
        manifests = check.typescript_manifests(check.ROOT)
        names = ("@baukit/integrations-client", "@baukit/localization-core", "@baukit/suite-client")
        packed_manifests = {name: manifests[name] for name in names}
        pending = list(packed_manifests.values())
        while pending:
            manifest = pending.pop()
            for name in json.loads(manifest.read_text()).get("dependencies", {}):
                if name in packed_manifests:
                    continue
                candidates = (
                    parent / "node_modules" / name / "package.json"
                    for parent in (manifest.parent, *manifest.parent.parents)
                )
                dependency = next((path.resolve() for path in candidates if path.is_file()), None)
                self.assertIsNotNone(dependency, f"install {name} before running package checks")
                packed_manifests[name] = dependency
                pending.append(dependency)
        with tempfile.TemporaryDirectory() as temporary:
            consumer = Path(temporary)
            archives = []
            # Include installed runtime dependencies without relying on npm's registry cache.
            for manifest in packed_manifests.values():
                packed = subprocess.run(
                    ["npm", "pack", "--json", "--ignore-scripts", "--pack-destination", temporary],
                    cwd=manifest.parent, capture_output=True, text=True,
                )
                self.assertEqual(packed.returncode, 0, packed.stdout + packed.stderr)
                archives.append(str(consumer / check.packed_filename(packed.stdout, consumer)))
            (consumer / "package.json").write_text(json.dumps({
                "name": "suite-client-consumer", "version": "0.0.0", "private": True,
            }))
            installed = subprocess.run(
                ["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", *archives],
                cwd=consumer, capture_output=True, text=True,
            )
            self.assertEqual(installed.returncode, 0, installed.stdout + installed.stderr)
            for name, manifest in packed_manifests.items():
                package = json.loads((consumer / "node_modules" / name / "package.json").read_text())
                self.assertEqual(package["name"], name)
                self.assertEqual(package["version"], json.loads(manifest.read_text())["version"])
            verified = subprocess.run(
                ["node", "--input-type=module", "-e", """
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { verifyTicket } from '@baukit/suite-client/entitlements';
const fixture = JSON.parse(readFileSync(process.argv[1], 'utf8'));
const vector = fixture.vectors[0];
assert.equal(vector.expected, 'ok');
assert.deepEqual(verifyTicket(vector.ticket, fixture.jwks, {
  issuer: fixture.issuer, audience: vector.audience, now: vector.now,
}), vector.claims);
""", str(check.ROOT / "fixtures/entitlement-tickets/v1/vectors.json")],
                cwd=consumer, capture_output=True, text=True,
            )
            self.assertEqual(verified.returncode, 0, verified.stdout + verified.stderr)
