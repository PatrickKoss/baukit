"""Every file a Rust build reads is a cache input of each step that builds it.

The gate cache trusts [quality.affected] in scripts/ci-gate.toml. This test finds build inputs
mechanically (include_str!/include_bytes!/include_dir! targets and Cargo path dependencies),
maps each package to the steps that run its manifest, and fails when an input would not
invalidate one of those steps.
"""

import re
import tomllib
import unittest
from collections.abc import Iterable
from pathlib import PurePosixPath

from process_runner import run_process
from test_gate_template import REPOSITORY, load_module

INCLUDE = re.compile(r"include_(?:str|bytes|dir)!\s*\(\s*\"([^\"]+)\"")
MANIFEST_FLAG = re.compile(r"--manifest-path\s+(\S*Cargo\.toml)")
STEP_FUNCTION = re.compile(r"^step_(\w+)\(\)\s*\{\n(.*?)^\}", re.M | re.S)
ONE_LINE_STEP = re.compile(r"^step_(\w+)\(\)\s*\{(.*)\}\s*$", re.M)

# Steps that build a manifest without naming it after --manifest-path in their own body.
INDIRECT_BUILDERS = {
    # `cd rust && cargo kani -p <crate>`, scripts/check-suite-features.sh, and the
    # scripts/vector-generators.toml commands (also parsed, pinned here against a rewrite).
    "rust/Cargo.toml": {"kani", "suite-features", "vectors"},
    # These steps build the CLI binary through scripts they call, or `cargo run` it.
    "cli/Cargo.toml": {
        "baukit-check",
        "fixture-1",
        "fixture-2",
        "affected-fixture",
        "mcp-fixture",
        "native-android",
    },
    # `cd rust/verus/equivalence && cargo kani` in step_verus.
    "rust/verus/equivalence/Cargo.toml": {"verus"},
}

LINKED_TABLES = ("dependencies", "build-dependencies", "workspace.dependencies")

# Steps that compile a workspace without its test targets, so an input read only from a
# crate's tests/ directory cannot change their result.
LIBRARY_ONLY_STEPS = {
    "kani",  # `cargo kani -p <crate>` compiles the library with cfg(kani).
    "vectors",  # Generators run examples and the one baukit-events test they name.
    "examples-test",  # Builds examples/minimal-api against the libraries only.
}

# Inputs that are not tracked repository files, with the reason they need no map entry.
UNTRACKED_INCLUDES = {
    # A string inside a generated test crate that the test writes to a temp dir.
    ("cli/tests/dockerfile.rs", "/generated/output.txt"),
}

# Inputs read outside the include macros.
EXTRA_INPUTS = {
    # cli/build.rs include_str!s every typescript/packages/*/package.json it lists at build
    # time and reruns on the pnpm lockfile.
    "cli": ["typescript/packages/*/package.json", "typescript/pnpm-lock.yaml"],
}


def tracked() -> list[str]:
    command = ["git", "-C", str(REPOSITORY), "ls-files"]
    return run_process(command, capture_output=True, text=True, check=True).stdout.splitlines()


def built_in_place(path: str) -> bool:
    # templates/ are rendered into products first; the CLI include_dir!s them whole.
    return not path.startswith("templates/")


def normalize(path: PurePosixPath) -> str:
    parts: list[str] = []
    for part in path.parts:
        if part == "..":
            parts.pop()
        elif part != ".":
            parts.append(part)
    return "/".join(parts)


def lookup(data: dict, dotted: str) -> dict:
    for key in dotted.split("."):
        data = data.get(key, {})
    return data


def is_test_only(package: str, source: str) -> bool:
    return source.startswith(f"{package}/tests/")


class Packages:
    def __init__(self, files: list[str]):
        self.files = files
        self.manifests = sorted(
            path for path in files if path.endswith("Cargo.toml") and built_in_place(path)
        )
        self.directories = {
            str(PurePosixPath(manifest).parent): manifest for manifest in self.manifests
        }

    def owner(self, path: str) -> str | None:
        directory = PurePosixPath(path).parent
        while True:
            if str(directory) in self.directories:
                return str(directory)
            if str(directory) == ".":
                return None
            directory = directory.parent

    def workspace(self, package: str) -> str:
        manifest = self.directories[package]
        data = tomllib.loads((REPOSITORY / manifest).read_text())
        if "workspace" in data:
            return manifest
        parent = self.owner(str(PurePosixPath(package).parent / "Cargo.toml"))
        while parent is not None:
            root = tomllib.loads((REPOSITORY / self.directories[parent]).read_text())
            members = root.get("workspace", {}).get("members", [])
            if any(normalize(PurePosixPath(parent) / member) == package for member in members):
                return self.directories[parent]
            parent = self.owner(str(PurePosixPath(parent).parent / "Cargo.toml"))
        return manifest

    def path_dependencies(self, package: str, keys: tuple[str, ...]) -> set[str]:
        data = tomllib.loads((REPOSITORY / self.directories[package]).read_text())
        inherited = self.inherited_dependencies(package)
        found = set()
        for key in keys:
            for name, spec in lookup(data, key).items():
                if isinstance(spec, dict) and spec.get("workspace") is True:
                    found.update(inherited.get(name, set()))
                elif isinstance(spec, dict) and "path" in spec:
                    found.add(normalize(PurePosixPath(package) / spec["path"]))
        return found - {package}

    def inherited_dependencies(self, package: str) -> dict[str, set[str]]:
        root_manifest = self.workspace(package)
        if root_manifest == self.directories[package]:
            return {}
        root = str(PurePosixPath(root_manifest).parent)
        data = tomllib.loads((REPOSITORY / root_manifest).read_text())
        return {
            name: {normalize(PurePosixPath(root) / spec["path"])}
            for name, spec in lookup(data, "workspace.dependencies").items()
            if isinstance(spec, dict) and "path" in spec
        }

    def files_of(self, directory: str) -> list[str]:
        prefix = f"{directory}/"
        return [path for path in self.files if path.startswith(prefix)]


def step_bodies(script: str) -> dict[str, str]:
    bodies = {name: body for name, body in ONE_LINE_STEP.findall(script)}
    bodies.update(dict(STEP_FUNCTION.findall(script)))
    return {name.replace("_", "-"): body for name, body in bodies.items()}


def manifest_builders(script: str) -> dict[str, set[str]]:
    builders: dict[str, set[str]] = {}
    for step, body in step_bodies(script).items():
        for manifest in MANIFEST_FLAG.findall(body):
            builders.setdefault(manifest, set()).add(step)
    generators = tomllib.loads((REPOSITORY / "scripts/vector-generators.toml").read_text())
    for generator in generators["generator"]:
        for manifest in MANIFEST_FLAG.findall(" ".join(generator["command"])):
            builders.setdefault(manifest, set()).add("vectors")
    for manifest, steps in INDIRECT_BUILDERS.items():
        builders.setdefault(manifest, set()).update(steps)
    return builders


def include_targets(packages: Packages, source: str) -> Iterable[str]:
    text = (REPOSITORY / source).read_text()
    package = packages.owner(source)
    for literal in INCLUDE.findall(text):
        if (source, literal) in UNTRACKED_INCLUDES or "{" in literal:
            continue
        if literal.startswith("$CARGO_MANIFEST_DIR/"):
            target = PurePosixPath(package) / literal.removeprefix("$CARGO_MANIFEST_DIR/")
        else:
            target = PurePosixPath(source).parent / literal
        yield normalize(target)


class BuildInputTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        results = load_module("gate_results")
        gate = load_module("gate")
        listing = run_process(
            ["sh", str(REPOSITORY / "scripts/ci-gate.sh"), "--list-steps"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        settings = gate.load_settings(REPOSITORY / "scripts/ci-gate.toml")
        cls.results = results
        cls.glob = staticmethod(gate.glob_pattern)
        cls.inputs = results.InputMap(
            settings["quality"]["affected"],
            gate.shared_paths(settings),
            gate.glob_pattern,
            gate.base_token,
        )
        cls.specs = {
            step.name: results.StepSpec(step.name, step.mode, step.tokens)
            for step in gate.parse_steps(listing)
        }
        cls.packages = Packages(tracked())
        script = (REPOSITORY / "scripts/ci-gate.sh").read_text()
        cls.builders = {
            manifest: steps & cls.specs.keys()
            for manifest, steps in manifest_builders(script).items()
        }

    def steps_seeing(self, path: str) -> set[str]:
        classes = self.results.classify([path], self.inputs)
        return {
            name for name, spec in self.specs.items() if self.results.step_inputs(spec, classes)
        }

    def package_builders(self) -> dict[str, set[str]]:
        """Steps that compile each package, following path dependencies to their dependents.

        A dev-dependency is compiled only by the dependent's own steps, so only normal and
        build edges pass a dependent's builders on transitively.
        """
        packages = self.packages
        direct = {
            package: set(self.builders.get(packages.workspace(package), set()))
            for package in packages.directories
        }
        linked: dict[str, set[str]] = {package: set() for package in packages.directories}
        tested: dict[str, set[str]] = {package: set() for package in packages.directories}
        for package in packages.directories:
            for dependency in packages.path_dependencies(package, LINKED_TABLES):
                linked.setdefault(dependency, set()).add(package)
            for dependency in packages.path_dependencies(package, ("dev-dependencies",)):
                tested.setdefault(dependency, set()).add(package)
        resolved: dict[str, set[str]] = {}

        def resolve(package: str, seen: frozenset[str]) -> set[str]:
            if package in resolved:
                return resolved[package]
            steps = set(direct.get(package, set()))
            for dependent in tested.get(package, set()):
                steps |= direct.get(dependent, set())
            for dependent in linked.get(package, set()) - seen:
                steps |= resolve(dependent, seen | {package})
            resolved[package] = steps
            return steps

        return {package: resolve(package, frozenset()) for package in packages.directories}

    def own_builders(self, package: str) -> set[str]:
        """Steps that compile the package's own test targets: those of its workspace."""
        return self.builders.get(self.packages.workspace(package), set()) - LIBRARY_ONLY_STEPS

    def package_inputs(self, package: str) -> dict[str, bool]:
        """Each file the package's build reads, mapped to whether only its tests read it."""
        files = self.packages.files_of(package) if package != "rust" else ["rust/Cargo.toml"]
        inputs = {path: is_test_only(package, path) for path in files}
        for source in files:
            if not source.endswith(".rs"):
                continue
            for target in include_targets(self.packages, source):
                for path in self.packages.files_of(target) or [target]:
                    inputs[path] = inputs.get(path, True) and is_test_only(package, source)
        for pattern in EXTRA_INPUTS.get(package, []):
            regex = self.glob(pattern)
            inputs.update({path: False for path in self.packages.files if regex.fullmatch(path)})
        return inputs

    def test_include_targets_exist(self):
        sources = (path for path in self.packages.files if path.endswith(".rs"))
        for source in filter(built_in_place, sources):
            for target in include_targets(self.packages, source):
                with self.subTest(source=source, target=target):
                    exists = (REPOSITORY / target).exists()
                    self.assertTrue(exists, f"{source} includes {target}, which does not exist")

    def test_every_build_input_invalidates_its_builders(self):
        gaps = []
        for package, steps in sorted(self.package_builders().items()):
            if not steps:
                continue
            for path, tests_only in sorted(self.package_inputs(package).items()):
                needed = self.own_builders(package) if tests_only else steps
                missing = needed - self.steps_seeing(path)
                if missing:
                    gaps.append(f"{path} (read by {package}) misses {sorted(missing)}")
        self.assertEqual(gaps, [], "\n".join(gaps[:40]))

    def test_vectors_reruns_for_its_generators_and_full_gates_for_its_outputs(self):
        generators = tomllib.loads((REPOSITORY / "scripts/vector-generators.toml").read_text())
        for generator in generators["generator"]:
            command = generator["command"]
            crate = command[command.index("-p") + 1]
            sources = (
                f"rust/crates/{crate}/src/lib.rs",
                "scripts/regenerate-vectors.py",
                "scripts/vector-generators.toml",
            )
            for source in sources:
                with self.subTest(generator=generator["output"], source=source):
                    self.assertIn("vectors", self.steps_seeing(source))
            classes = self.results.classify([generator["output"]], self.inputs)
            self.assertIsNone(classes[generator["output"]], "outputs are shared")

    def test_known_edges_are_found(self):
        builders = self.package_builders()
        self.assertIn("verus", builders["rust/crates/baukit-sync"])
        self.assertIn("vectors", builders["rust/crates/baukit-core"])
        self.assertIn("examples-test", builders["rust/crates/baukit-http"])
        self.assertIn("lint/agent-rules.md", self.package_inputs("cli"))
        self.assertIn("deploy/observability/lint/check-metric-names.py", self.package_inputs("cli"))


if __name__ == "__main__":
    unittest.main()
