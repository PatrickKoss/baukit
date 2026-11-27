import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import threading
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))


def load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


results = load("gate_results")
gate = load("gate")

AFFECTED = {
    "docs/**": [],
    "backend/**": ["backend"],
    "web/**": ["web", "playwright:@smoke"],
}
SHARED = ["Cargo.lock"]
INPUTS = results.InputMap(AFFECTED, SHARED, gate.glob_pattern, gate.base_token)
STEPS = [
    results.StepSpec("rust-tests", "selected", frozenset({"rust-tests", "backend"})),
    results.StepSpec("playwright", "selected", frozenset({"playwright", "web"})),
    results.StepSpec("lint", "always", frozenset({"lint", "fast"})),
]
DAY = results.SECONDS_PER_DAY


def git_in(root):
    def git(*arguments):
        result = subprocess.run(
            ["git", "-C", str(root), *arguments], capture_output=True, text=True, check=False
        )
        return result.stdout.strip() if result.returncode == 0 else None

    return git


class Repository:
    def __init__(self, root):
        self.root = root
        self.git = git_in(root)
        root.mkdir(parents=True, exist_ok=True)
        for arguments in (
            ["init", "-q"],
            ["config", "user.email", "gate@example.invalid"],
            ["config", "user.name", "gate"],
        ):
            self.git(*arguments)
        self.write("settings.toml", "")
        for path in ("docs/a.md", "backend/lib.rs", "web/app.ts", "Cargo.lock", "README"):
            self.write(path, path)
        self.commit()

    def write(self, path, text):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(self):
        self.git("add", "-A")
        self.git("commit", "-qm", "change")

    def request(self, settings=None, environment=None, no_cache=False):
        machinery = results.Machinery(self.root / "settings.toml", ())
        environment = {"XDG_CACHE_HOME": str(self.root / "cache"), **(environment or {})}
        return results.CacheRequest(STEPS, settings or {}, INPUTS, machinery, environment, no_cache)

    def open(self, clock=None, **request):
        cache, line = results.open_cache(
            self.request(**request), self.git, clock or (lambda: 1000.0)
        )
        return cache, line

    def keys(self, **request):
        cache, line = self.open(**request)
        self_check = cache is not None
        assert self_check, line
        return cache.keys


class KeyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.repository = Repository(Path(self.directory.name))

    def tearDown(self):
        self.directory.cleanup()

    def changed_keys(self, path, text="changed"):
        before = self.repository.keys()
        self.repository.write(path, text)
        self.repository.commit()
        after = self.repository.keys()
        return {name for name in before if before[name] != after[name]}

    def test_keys_are_stable(self):
        self.assertEqual(self.repository.keys(), self.repository.keys())

    def test_a_file_selected_by_a_token_changes_only_its_steps(self):
        self.assertEqual(self.changed_keys("backend/lib.rs"), {"rust-tests", "lint"})

    def test_a_tagged_token_counts_as_its_base_token(self):
        self.assertEqual(self.changed_keys("web/app.ts"), {"playwright", "lint"})

    def test_a_shared_file_changes_every_key(self):
        self.assertEqual(self.changed_keys("Cargo.lock"), {"rust-tests", "playwright", "lint"})

    def test_a_file_matching_no_glob_changes_every_key(self):
        self.assertEqual(self.changed_keys("README"), {"rust-tests", "playwright", "lint"})

    def test_a_new_unmatched_file_changes_every_key(self):
        self.assertEqual(self.changed_keys("new.txt"), {"rust-tests", "playwright", "lint"})

    def test_an_unrelated_file_changes_only_the_always_steps(self):
        self.assertEqual(self.changed_keys("docs/a.md"), {"lint"})

    def test_the_machinery_changes_every_key(self):
        self.assertEqual(
            self.changed_keys("settings.toml", "[quality]\n"), {"rust-tests", "playwright", "lint"}
        )

    def test_the_run_context_changes_every_key(self):
        before = self.repository.keys()
        for environment in (
            {"BAUKIT_GATE_MODE": "affected"},
            {"BAUKIT_GATE_TOKENS": "backend"},
            {"BAUKIT_GATE_RUST_PACKAGES": "core"},
            {"BAUKIT_FUZZ_SECONDS": "1"},
        ):
            after = self.repository.keys(environment=environment)
            self.assertTrue(all(before[name] != after[name] for name in before), environment)

    def test_host_sharing_variables_leave_the_keys_alone(self):
        environment = {"BAUKIT_GATE_CORES": "3", "BAUKIT_GATE_LOG_DIR": "/x", "CARGO_HOME": "/y"}
        self.assertEqual(self.repository.keys(), self.repository.keys(environment=environment))

    def test_the_base_resolves_to_a_commit(self):
        head = self.repository.git("rev-parse", "HEAD")
        context = results.run_context({"BAUKIT_GATE_BASE": "HEAD"}, self.repository.git)
        self.assertEqual(context, [f"BAUKIT_GATE_BASE={head}"])

    def test_never_steps_get_no_key(self):
        settings = {"quality": {"cache": {"never": ["rust-tests"]}}}
        cache, line = self.repository.open(settings=settings)
        self.assertNotIn("rust-tests", cache.keys)
        self.assertIsNone(cache.lookup("rust-tests"))
        self.assertFalse(cache.store("rust-tests", 5, Path("log")))
        self.assertIn("never cached: rust-tests", line)


class SwitchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.repository = Repository(Path(self.directory.name))

    def tearDown(self):
        self.directory.cleanup()

    def assert_off(self, reason, **request):
        cache, line = self.repository.open(**request)
        self.assertIsNone(cache)
        self.assertEqual(line, f"cache: off ({reason}); every step runs")

    def test_untracked_files_turn_the_cache_off(self):
        self.repository.write("scratch.txt", "x")
        self.assert_off("the tree has uncommitted or untracked files")

    def test_modified_files_turn_the_cache_off(self):
        self.repository.write("backend/lib.rs", "edited")
        self.assert_off("the tree has uncommitted or untracked files")

    def test_no_cache_flag(self):
        self.assert_off("--no-cache", no_cache=True)

    def test_ci(self):
        self.assert_off("CI is set", environment={"CI": "true"})

    def test_environment_switch(self):
        self.assert_off("BAUKIT_GATE_CACHE=off", environment={"BAUKIT_GATE_CACHE": "off"})

    def test_a_path_dependency_outside_the_repository(self):
        settings = {"dependencies": {"baukit": {"source": "path", "path": "../baukit/rust"}}}
        self.assert_off(
            "dependencies.baukit is a path source outside this repository", settings=settings
        )


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.repository = Repository(Path(self.directory.name))
        self.now = 1000.0
        self.cache, _ = self.repository.open(clock=lambda: self.now)

    def tearDown(self):
        self.directory.cleanup()

    def test_a_stored_pass_is_found_until_it_expires(self):
        self.assertIsNone(self.cache.lookup("rust-tests"))
        self.assertTrue(self.cache.store("rust-tests", 42, Path("logs/backend.log")))
        entry = self.cache.lookup("rust-tests")
        self.assertEqual((entry.step, entry.seconds), ("rust-tests", 42))
        self.assertEqual(entry.commit, self.repository.git("rev-parse", "HEAD"))
        self.now += results.DEFAULT_MAX_AGE_DAYS * DAY
        self.assertIsNotNone(self.cache.lookup("rust-tests"))
        self.now += 1
        self.assertIsNone(self.cache.lookup("rust-tests"))

    def test_max_age_comes_from_the_settings(self):
        cache, _ = self.repository.open(
            settings={"quality": {"cache": {"max_age_days": 1}}}, clock=lambda: self.now
        )
        cache.store("lint", 1, Path("log"))
        self.now += DAY + 1
        self.assertIsNone(cache.lookup("lint"))

    def test_entries_are_shared_by_worktrees_of_one_repository(self):
        self.cache.store("lint", 1, Path("log"))
        worktree = Path(self.directory.name) / "worktree"
        self.repository.git("worktree", "add", "-q", "--detach", str(worktree))
        other = Repository.__new__(Repository)
        other.root, other.git = worktree, git_in(worktree)
        request = self.repository.request()
        cache, line = results.open_cache(
            results.CacheRequest(
                request.steps,
                {},
                INPUTS,
                results.Machinery(worktree / "settings.toml", ()),
                request.environment,
            ),
            other.git,
            lambda: self.now,
        )
        self.assertIsNotNone(cache, line)
        self.assertEqual(cache.directory, self.cache.directory)
        self.assertIsNotNone(cache.lookup("lint"))

    def test_a_tree_that_changed_during_the_run_is_not_stored(self):
        self.repository.write("backend/lib.rs", "edited")
        self.assertFalse(self.cache.store("rust-tests", 1, Path("log")))
        self.assertIn("rust-tests not stored", self.cache.notes[0])
        self.assertIsNone(self.cache.lookup("rust-tests"))

    def test_a_moved_head_is_not_stored(self):
        self.repository.write("docs/a.md", "edited")
        self.repository.commit()
        self.assertFalse(self.cache.store("lint", 1, Path("log")))

    def test_a_corrupt_entry_is_a_miss(self):
        path = self.cache.entry_path("lint")
        path.parent.mkdir(parents=True)
        path.write_text("{not json")
        self.assertIsNone(self.cache.lookup("lint"))

    def test_concurrent_writes_leave_a_valid_file(self):
        def write(seconds):
            for _ in range(20):
                results.write_entry(
                    self.cache.entry_path("lint"),
                    results.Entry("lint", self.cache.keys["lint"], "abc", 1.0, seconds, "log"),
                )

        threads = [threading.Thread(target=write, args=(seconds,)) for seconds in range(8)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        data = json.loads(self.cache.entry_path("lint").read_text())
        self.assertIn(data["seconds"], range(8))
        self.assertEqual(list(self.cache.directory.glob(".*.tmp")), [])


class ProviderTests(unittest.TestCase):
    class FakeCache:
        def __init__(self, names):
            self.names = names

        def lookup(self, name):
            return results.Entry(name, "k", "abc", 0.0, 1, "log") if name in self.names else None

    def test_a_running_step_keeps_its_providers_running(self):
        after = {"install": (), "check": ("install",), "fixture": ("check",), "lint": ()}
        cached = results.cached_steps(self.FakeCache({"install", "check", "lint"}), after)
        self.assertEqual(set(cached), {"lint"})

    def test_cached_dependents_keep_cached_providers(self):
        after = {"install": (), "check": ("install",)}
        cached = results.cached_steps(self.FakeCache({"install", "check"}), after)
        self.assertEqual(set(cached), {"install", "check"})


class SettingsTests(unittest.TestCase):
    def test_unknown_never_names_and_bad_ages_are_errors(self):
        self.assertIsNone(results.settings_error({}, ["a"]))
        never = {"quality": {"cache": {"never": ["b"]}}}
        self.assertEqual(
            results.settings_error(never, ["a"]),
            "gate: quality.cache.never names unknown steps: b",
        )
        age = {"quality": {"cache": {"max_age_days": "7"}}}
        self.assertIn("max_age_days", results.settings_error(age, ["a"]))

    def test_age_text(self):
        self.assertEqual(results.age_text(5), "5 s ago")
        self.assertEqual(results.age_text(125), "2 min ago")
        self.assertEqual(results.age_text(7200), "2 h ago")
        self.assertEqual(results.age_text(3 * DAY), "3 d ago")


class GateRunTests(unittest.TestCase):
    STEPS = (
        "install fast - always -",
        "build web install selected web",
        "unit web build selected -",
    )

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.repository = Repository(self.root / "repo")
        self.repository.write(".gitignore", "logs/\nran\n")
        self.repository.write(
            "settings.toml",
            '[quality.affected]\n"docs/**" = []\n"web/**" = ["web"]\n"backend/**" = ["backend"]\n',
        )
        self.repository.write("steps.sh", self.script())
        self.repository.commit()

    def tearDown(self):
        self.directory.cleanup()

    def script(self):
        lines = " ".join(repr(line) for line in self.STEPS)
        return textwrap.dedent(f"""\
            if [ "$1" = --list-steps ]; then
              printf '%s\\n' {lines}; exit 0
            fi
            echo "$2" >> ran
            [ "$2" != unit ] || [ ! -f fail-unit ] || exit 3
        """)

    def run_gate(self, *extra, environment=None):
        ran = self.repository.root / "ran"
        ran.unlink(missing_ok=True)
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "gate.py"),
                "run",
                "--script",
                "sh steps.sh",
                "--settings",
                "settings.toml",
                "--log-dir",
                "logs",
                *extra,
            ],
            cwd=self.repository.root,
            capture_output=True,
            text=True,
            check=False,
            env={
                "PATH": os.environ["PATH"],
                "HOME": str(self.root),
                "XDG_CACHE_HOME": str(self.root / "cache"),
                "PYTHONPATH": os.pathsep.join(sys.path),
                "PYTHONDONTWRITEBYTECODE": "1",
                "BAUKIT_GATE_CORES": "2",
                "BAUKIT_GATE_DISK_FLOOR_GB": "0",
                **(environment or {}),
            },
        )
        order = ran.read_text().split() if ran.exists() else []
        return result, order

    def test_a_second_run_skips_passed_steps_until_an_input_changes(self):
        first, order = self.run_gate()
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(order, ["install", "build", "unit"])
        self.assertIn("gate: unit pass stored in the result cache", first.stdout)

        plan, order = self.run_gate("--plan")
        self.assertEqual(order, [])
        self.assertRegex(plan.stdout, r"gate:   web/unit \(full gate; cached, passed at \w{7}, ")

        second, order = self.run_gate()
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
        self.assertEqual(order, [])
        self.assertRegex(second.stdout, r"gate: \[web\] unit cached \(passed at \w{7}, \d+ s ago\)")
        self.assertIn("gate: steps", second.stdout)
        history = (self.repository.root / "logs/step-seconds.txt").read_text()
        self.assertRegex(history, r"(?m)^unit [1-9]\d*$")
        timings = (self.repository.root / "logs/timings.txt").read_text()
        self.assertNotRegex(timings, r"(?m)^unit ")

        self.repository.write("web/app.ts", "edited")
        self.repository.commit()
        third, order = self.run_gate()
        self.assertEqual(third.returncode, 0, third.stdout + third.stderr)
        self.assertEqual(order, ["install", "build", "unit"])

        self.repository.write("backend/lib.rs", "edited")
        self.repository.commit()
        fourth, order = self.run_gate()
        self.assertEqual(order, ["install"])
        self.assertRegex(fourth.stdout, r"\[web\] build cached")

    def test_failures_are_not_stored(self):
        self.repository.write("fail-unit", "")
        self.repository.write(".gitignore", "logs/\nran\nfail-unit\n")
        self.repository.commit()
        first, order = self.run_gate()
        self.assertEqual(first.returncode, 1)
        self.assertEqual(order, ["install", "build", "unit"])
        stored = sorted(path.name.partition("-")[0] for path in self.root.glob("cache/**/*.json"))
        self.assertEqual(stored, ["build", "install"])
        second, order = self.run_gate()
        self.assertEqual(second.returncode, 1)
        self.assertEqual(order, ["install", "build", "unit"])

    def test_no_cache_and_ci_run_every_step(self):
        self.run_gate()
        for extra, environment in ((("--no-cache",), None), ((), {"CI": "true"})):
            result, order = self.run_gate(*extra, environment=environment)
            self.assertEqual(order, ["install", "build", "unit"])
            self.assertIn("cache: off (", result.stdout)

    def test_a_dirty_tree_neither_reads_nor_writes(self):
        self.run_gate()
        self.repository.write("scratch.txt", "x")
        result, order = self.run_gate()
        self.assertEqual(order, ["install", "build", "unit"])
        self.assertIn("cache: off (the tree has uncommitted or untracked files)", result.stdout)
        self.assertNotIn("stored in the result cache", result.stdout)


if __name__ == "__main__":
    unittest.main()
