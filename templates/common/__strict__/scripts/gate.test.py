import importlib.util
import subprocess
import sys
import tempfile
import textwrap
import threading
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().with_name("gate.py")
spec = importlib.util.spec_from_file_location("gate", SCRIPT)
gate = importlib.util.module_from_spec(spec)
sys.modules["gate"] = gate
spec.loader.exec_module(gate)

STEPS = gate.parse_steps(
    textwrap.dedent("""
    rust-fmt fast - always -
    openapi fast - always -
    rust-clippy backend - always -
    rust-tests backend rust-clippy selected -
    images images - selected -
    web-build web openapi selected web-e2e
    web-unit web web-build selected -
    playwright web web-build selected web-e2e,playwright
    mobile-unit mobile - selected -
    android-e2e mobile mobile-unit selected native
    drift-only fast openapi affected -
""")
)

AFFECTED = {
    "backend/crates/**": ["backend"],
    "web/src/**": ["web"],
    "web/e2e/tests/**": ["web-e2e"],
    "mobile/src/**": ["mobile-unit"],
    "mobile/app.config.ts": ["mobile"],
    "docs/**": [],
}

SHARED = list(gate.GENERIC_SHARED) + ["backend/openapi.json"]


def names(planned):
    return [item.step.name for item in planned]


class MatcherTests(unittest.TestCase):
    def test_globs(self):
        self.assertTrue(gate.matches("backend/crates/**", "backend/crates/a/src/lib.rs"))
        self.assertTrue(gate.matches("**/Cargo.lock", "Cargo.lock"))
        self.assertTrue(gate.matches("**/Cargo.lock", "backend/Cargo.lock"))
        self.assertTrue(gate.matches("web/*.ts", "web/vite.config.ts"))
        self.assertFalse(gate.matches("web/*.ts", "web/src/main.ts"))
        self.assertFalse(gate.matches("docs/**", "docsx/a.md"))

    def test_one_crate_change_selects_backend(self):
        selection = gate.select(["backend/crates/a/src/lib.rs"], AFFECTED, SHARED)
        self.assertFalse(selection.full)
        self.assertEqual(
            names(gate.plan(STEPS, selection)),
            ["rust-fmt", "openapi", "rust-clippy", "rust-tests", "drift-only"],
        )

    def test_web_only_change_skips_backend_and_mobile(self):
        selection = gate.select(["web/src/App.tsx"], AFFECTED, SHARED)
        self.assertEqual(
            names(gate.plan(STEPS, selection)),
            [
                "rust-fmt",
                "openapi",
                "rust-clippy",
                "web-build",
                "web-unit",
                "playwright",
                "drift-only",
            ],
        )

    def test_spec_change_selects_only_browser_steps(self):
        selection = gate.select(["web/e2e/tests/qa-axe.spec.ts"], AFFECTED, SHARED)
        self.assertEqual(
            names(gate.plan(STEPS, selection)),
            ["rust-fmt", "openapi", "rust-clippy", "web-build", "playwright", "drift-only"],
        )

    def test_mobile_source_change_skips_android(self):
        selection = gate.select(["mobile/src/screen.tsx"], AFFECTED, SHARED)
        self.assertNotIn("android-e2e", names(gate.plan(STEPS, selection)))
        self.assertIn("mobile-unit", names(gate.plan(STEPS, selection)))

    def test_native_configuration_change_runs_android(self):
        selection = gate.select(["mobile/app.config.ts"], AFFECTED, SHARED)
        self.assertIn("android-e2e", names(gate.plan(STEPS, selection)))

    def test_maestro_only_change_selects_the_android_steps_in_any_lane(self):
        steps = gate.parse_steps(
            textwrap.dedent("""
            mobile-unit mobile - selected -
            android-build native mobile-unit selected -
            android-e2e native android-build selected -
        """)
        )
        affected = {"mobile/.maestro/**": ["mobile", "android-build", "android-e2e"]}
        selection = gate.select(["mobile/.maestro/smoke.yaml"], affected, SHARED)
        self.assertEqual(
            names(gate.plan(steps, selection)), ["mobile-unit", "android-build", "android-e2e"]
        )

    def test_shared_paths_run_the_full_gate(self):
        for path in (
            "backend/Cargo.lock",
            "web/pnpm-lock.yaml",
            "backend/migrations/0002_x.sql",
            "baukit.toml",
            "limits.json",
            "scripts/quality-gate.sh",
            "backend/openapi.json",
        ):
            with self.subTest(path=path):
                selection = gate.select(["web/src/App.tsx", path], AFFECTED, SHARED)
                self.assertTrue(selection.full)
                self.assertIn("shared path", selection.reasons[0])
                self.assertNotIn("drift-only", names(gate.plan(STEPS, selection)))
                self.assertIn("android-e2e", names(gate.plan(STEPS, selection)))

    def test_unmatched_path_runs_the_full_gate(self):
        selection = gate.select(["README.md"], AFFECTED, SHARED)
        self.assertTrue(selection.full)
        self.assertIn("matches no quality.affected glob", selection.reasons[0])

    def test_docs_only_change_runs_fast_checks(self):
        selection = gate.select(["docs/a.md"], AFFECTED, SHARED)
        self.assertEqual(
            names(gate.plan(STEPS, selection)), ["rust-fmt", "openapi", "rust-clippy", "drift-only"]
        )

    def test_unknown_tokens_are_reported(self):
        self.assertEqual(
            gate.unknown_tokens({"a/**": ["backend", "playwright:@auth", "nope"]}, STEPS), ["nope"]
        )

    def test_shared_paths_include_openapi_and_extra_entries(self):
        settings = {
            "openapi": {"schema": "backend/openapi.json", "consumers": ["web/src/api.d.ts"]},
            "quality": {"shared": ["deploy/**"]},
        }
        paths = gate.shared_paths(settings)
        for expected in (
            "backend/openapi.json",
            "backend/mcp-tools.json",
            "web/src/api.d.ts",
            "deploy/**",
        ):
            self.assertIn(expected, paths)


class StepTableTests(unittest.TestCase):
    def test_rejects_unknown_dependencies_and_modes(self):
        with self.assertRaises(ValueError):
            gate.parse_steps("a fast b always -\n")
        with self.assertRaises(ValueError):
            gate.parse_steps("a fast - sometimes -\n")
        with self.assertRaises(ValueError):
            gate.parse_steps("a fast - always -\na web - always -\n")


class PackageTests(unittest.TestCase):
    def metadata(self, root):
        def package(name, directory, dependencies=()):
            return {
                "id": name,
                "name": name,
                "manifest_path": str(root / directory / "Cargo.toml"),
                "dependencies": [
                    {"name": dependency, "path": str(root / dependency)}
                    for dependency in dependencies
                ],
            }

        packages = [
            package("domain", "backend/crates/domain"),
            package("services", "backend/crates/services", ["domain"]),
            package("api", "backend/crates/api", ["services"]),
            package("tools", "backend/crates/tools"),
        ]
        return {"packages": packages, "workspace_members": [item["id"] for item in packages]}

    def test_reverse_dependencies_follow_the_changed_crate(self):
        root = Path("/repo")
        members, dependents = gate.workspace_members(self.metadata(root), root)
        self.assertEqual(
            gate.affected_packages(
                ["backend/crates/domain/src/lib.rs"], "backend", members, dependents
            ),
            ["api", "domain", "services"],
        )
        self.assertEqual(
            gate.affected_packages(
                ["backend/crates/tools/src/lib.rs"], "backend", members, dependents
            ),
            ["tools"],
        )
        self.assertIsNone(
            gate.affected_packages(["backend/Cargo.toml"], "backend", members, dependents)
        )


class WorkerTests(unittest.TestCase):
    def test_cores_split_across_heavy_lanes(self):
        self.assertEqual(gate.split_workers(12, 4, None, 1024), 3)
        self.assertEqual(gate.split_workers(12, 0, None, 1024), 12)
        self.assertEqual(gate.split_workers(2, 4, None, 1024), 1)

    def test_total_memory_sets_a_floor_per_worker(self):
        self.assertEqual(gate.split_workers(24, 2, 8192, 1024), 4)
        self.assertEqual(gate.split_workers(24, 2, 65536, 1024), 12)

    def test_heavy_lanes_skip_fast_checks(self):
        planned = gate.plan(STEPS, gate.Selection(True, []))
        self.assertEqual(gate.heavy_lanes(planned), ["backend", "images", "web", "mobile"])

    def test_total_memory_reads_meminfo(self):
        with tempfile.TemporaryDirectory() as directory:
            meminfo = Path(directory) / "meminfo"
            meminfo.write_text("MemTotal:       47185920 kB\nCommitted_AS:   99999999 kB\n")
            self.assertEqual(gate.total_memory_mb(meminfo), 46080)


class BudgetTests(unittest.TestCase):
    def test_within_budget(self):
        report = gate.budget_report(
            {"a": 5, "b": 9}, {"x": ["a", "b"]}, {"x": 14}, 14, gate.BudgetLimit(30, "full")
        )
        self.assertEqual(report[1].split(), ["b", "9", "s"])
        self.assertIn("within the 30 minute full budget", report[-1])

    def test_over_budget_warns_with_the_slowest_lane(self):
        timings = {"rust-tests": 900, "rust-clippy": 120, "web-build": 60}
        lanes = {"backend": ["rust-clippy", "rust-tests"], "web": ["web-build"]}
        report = gate.budget_report(
            timings, lanes, {"backend": 1020, "web": 60}, 1020, gate.BudgetLimit(15, "affected")
        )
        self.assertIn(
            "gate: warning: 1020 s is over the 15 minute affected budget by 120 s", report
        )
        self.assertIn(
            "gate: warning: lane backend set the wall time: rust-tests 900 s, rust-clippy 120 s",
            report,
        )

    def test_budget_defaults(self):
        self.assertEqual(gate.budget_minutes({}, "full"), 10)
        self.assertEqual(gate.budget_minutes({}, "affected"), 5)
        self.assertEqual(
            gate.budget_minutes({"quality": {"gate_budget_minutes": {"full": 40}}}, "full"), 40
        )


class WeightedShareTests(unittest.TestCase):
    def runner(self, history, cores=24):
        directory = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(directory, ignore_errors=True))
        if history is not None:
            (directory / "step-seconds.txt").write_text(history)
        planned = gate.plan(STEPS, gate.Selection(True, []))
        budget = gate.CoreBudget(cores, None, 1024, 2)
        context = gate.RunContext([], directory, {})
        return gate.Runner(planned, context, budget=budget)

    def test_no_timings_keep_the_even_share(self):
        runner = self.runner(None)
        lanes = len(runner.open_lanes)
        self.assertEqual(
            runner.step_share(next(i.step for i in runner.planned if i.step.name == "playwright")),
            24 // lanes,
        )

    def test_long_pole_gets_the_bigger_share(self):
        runner = self.runner("playwright 1500\nweb-build 100\nrust-tests 200\nmobile-unit 200\n")
        by_name = {item.step.name: item.step for item in runner.planned}
        even = 24 // len(runner.open_lanes)
        self.assertGreater(runner.step_share(by_name["playwright"]), even)
        self.assertEqual(runner.step_share(by_name["web-build"]), even)

    def test_share_never_exceeds_free_cores(self):
        runner = self.runner("playwright 1500\nweb-build 1\n")
        runner.budget.acquire(20)
        even = 24 // len(runner.open_lanes)
        by_name = {item.step.name: item.step for item in runner.planned}
        self.assertEqual(runner.step_share(by_name["playwright"]), even)

    def test_share_never_exceeds_the_budget(self):
        runner = self.runner("playwright 1500\n", cores=3)
        by_name = {item.step.name: item.step for item in runner.planned}
        self.assertLessEqual(runner.step_share(by_name["playwright"]), 3)

    def test_history_survives_partial_and_failed_runs(self):
        runner = self.runner("playwright 1500\n")
        runner.history_file.write_text("playwright 1500\nweb-build 100\n")
        runner = gate.Runner(
            runner.planned,
            gate.RunContext([], runner.log_directory, {}),
            budget=runner.budget,
        )
        runner.done["rust-fmt"] = threading.Event()
        runner.record("rust-fmt", 7, "passed")
        runner.record("playwright", 9, "failed with exit 1")
        saved = gate.read_expected_seconds(runner.history_file)
        self.assertEqual(saved, {"playwright": 1500, "web-build": 100, "rust-fmt": 7})
        self.assertEqual(
            sorted(p.name for p in runner.log_directory.iterdir() if p.name.endswith(".tmp")), []
        )

    def test_malformed_and_stale_lines_are_ignored(self):
        text = "playwright abc\nplaywright -5\ngone 900\nlane:web 50\ntotal 99\n\nweb-build\nweb-unit 0\n"
        runner = self.runner(text)
        self.assertEqual(runner.expected, {})
        directory = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(directory, ignore_errors=True))
        (directory / "timings.txt").write_bytes(b"\xff\xfe")
        self.assertEqual(gate.read_expected_seconds(directory / "timings.txt", {"a"}), {})
        self.assertEqual(gate.read_expected_seconds(directory / "missing.txt", {"a"}), {})


class CoreBudgetTests(unittest.TestCase):
    def test_heavy_steps_never_hold_more_than_the_budget(self):
        budget = gate.CoreBudget(4, None, 1024, 2)
        self.assertEqual(budget.acquire(3), 3)
        self.assertEqual(budget.acquire(3), 1)
        released = threading.Timer(0.1, budget.release, args=(3,))
        released.start()
        self.assertEqual(budget.acquire(2), 2)
        released.join()

    def test_emulator_steps_wait_for_their_reserve(self):
        budget = gate.CoreBudget(6, None, 1024, 4)
        self.assertEqual(budget.acquire(3), 3)
        granted = []
        waiter = threading.Thread(target=lambda: granted.append(budget.acquire(4, 4)))
        waiter.start()
        waiter.join(0.1)
        self.assertEqual(granted, [])
        budget.release(3)
        waiter.join(1)
        self.assertEqual(granted, [4])

    def test_reserve_and_requests_stay_within_the_budget(self):
        budget = gate.CoreBudget(2, None, 1024, 8)
        self.assertEqual(budget.emulator_cores, 2)
        self.assertEqual(budget.acquire(9, 9), 2)

    def test_emulator_cores_follow_the_emulator_count(self):
        self.assertEqual(gate.emulator_cores({}, {}), 4)
        self.assertEqual(gate.emulator_cores({"quality": {"android_emulators": 3}}, {}), 6)
        self.assertEqual(gate.emulator_cores({}, {"BAUKIT_QA_ANDROID_EMULATORS": "1"}), 2)
        self.assertEqual(gate.emulator_cores({}, {"BAUKIT_GATE_EMULATOR_CORES": "5"}), 5)

    def test_light_lanes_and_steps_take_no_share(self):
        planned = gate.plan(STEPS, gate.Selection(True, []))
        light = frozenset({"images", "web-unit"})
        self.assertEqual(gate.heavy_lanes(planned, light), ["backend", "web", "mobile"])
        self.assertEqual(
            gate.heavy_lanes(planned, frozenset({"images", "mobile"})), ["backend", "web"]
        )
        self.assertEqual(gate.unknown_light(frozenset({"images", "nope"}), STEPS), ["nope"])


class RunnerTests(unittest.TestCase):
    def run_gate(self, steps, failing, *extra, settings_text="", script_body=""):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "steps.sh"
            script.write_text(
                textwrap.dedent(f"""\
                if [ "$1" = --list-steps ]; then
                  printf '%s\\n' {" ".join(repr(line) for line in steps)}; exit 0
                fi
                echo "ran $2" >> "{root}/ran"
                case " {" ".join(failing)} " in *" $2 "*) exit 3 ;; esac
            """)
                + script_body
            )
            settings = root / "settings.toml"
            settings.write_text(settings_text)
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "run",
                    "--script",
                    f"sh {script}",
                    "--settings",
                    str(settings),
                    "--log-dir",
                    str(root / "logs"),
                    *extra,
                ],
                capture_output=True,
                text=True,
                check=False,
                env={"PATH": "/usr/bin:/bin", "BAUKIT_GATE_CORES": "4"},
            )
            ran = (root / "ran").read_text().split("\n") if (root / "ran").exists() else []
            timings = (
                (root / "logs/timings.txt").read_text()
                if (root / "logs/timings.txt").exists()
                else ""
            )
            lanes = (
                sorted(path.name for path in (root / "logs").glob("*.log"))
                if (root / "logs").exists()
                else []
            )
            return result, ran, timings, lanes

    def test_lanes_run_and_write_timings_and_logs(self):
        result, ran, timings, lanes = self.run_gate(
            ["a fast - always -", "b web a selected -", "c mobile - selected -"], []
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertLess(ran.index("ran a"), ran.index("ran b"))
        self.assertIn("ran c", ran)
        for name in ("a", "b", "c"):
            self.assertRegex(timings, rf"(?m)^{name} \d+$")
        self.assertRegex(timings, r"(?m)^total \d+$")
        self.assertEqual(lanes, ["fast.log", "mobile.log", "web.log"])
        self.assertIn("gate: step timings, slowest first", result.stdout)
        self.assertIn("workers: 4 cores across 2 lanes (web, mobile)", result.stdout)

    def test_failed_lane_is_listed_and_dependents_do_not_run(self):
        result, ran, _, _ = self.run_gate(
            ["a fast - always -", "b web a selected -", "c mobile - selected -"], ["a"]
        )
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("ran b", ran)
        self.assertIn("ran c", ran)
        self.assertIn(
            "gate: failed lanes: fast (a failed with exit 3), web (b blocked by a)", result.stdout
        )

    def test_running_steps_share_one_budget(self):
        body = textwrap.dedent("""\
            echo "env $2 $BAUKIT_GATE_WORKERS $BAUKIT_QA_GRADLE_WORKERS" \\
              "$BAUKIT_QA_NATIVE_JOBS" >> "$(dirname "$0")/ran"
            sleep 0.3
        """)
        result, ran, _, _ = self.run_gate(
            [
                "a fast - always -",
                "b web - selected -",
                "c mobile - selected -",
                "android-e2e mobile c selected -",
                "d docs - selected -",
            ],
            [],
            settings_text='[quality]\nlight = ["docs"]\nandroid_emulators = 1\n',
            script_body=body,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("env b 2 2 2", ran)
        self.assertIn("gate: [web] b started with 2 cores", result.stdout)
        self.assertIn("gate: [mobile] c started with 2 cores", result.stdout)
        self.assertIn("gate: [docs] d started with 2 cores, light", result.stdout)
        self.assertRegex(result.stdout, r"gate: \[mobile\] android-e2e started with [2-4] cores")
        self.assertIn("keeps 2 for the emulators; light: docs, fast", result.stdout)
        self.assertRegex(
            result.stdout,
            r"gate: \[web\] b started with 2 cores, at \+\d+ s after waiting \d+ s, "
            r"\d+ of \d+ cores free",
        )

    def test_unknown_light_names_fail(self):
        result, ran, _, _ = self.run_gate(
            ["a fast - always -"], [], settings_text='[quality]\nlight = ["nope"]\n'
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("quality.light names unknown lanes or steps: nope", result.stderr)
        self.assertEqual(ran, [])

    def test_plan_prints_steps_without_running(self):
        result, ran, _, _ = self.run_gate(["a fast - always -"], [], "--plan")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(ran, [])
        self.assertIn("gate:   fast/a (full gate)", result.stdout)


class SummaryTests(unittest.TestCase):
    def test_unrun_steps_are_listed_as_not_run(self):
        with tempfile.TemporaryDirectory() as directory:
            step = gate.Step("a", "fast", (), "always", frozenset({"a", "fast"}))
            context = gate.RunContext(["missing-gate-binary"], Path(directory), {})
            runner = gate.Runner([gate.PlannedStep(step, "full gate")], context)
            runner.status.clear()
            runner.run_lane = lambda lane, steps: None
            failures = runner.run()
        self.assertEqual(failures, ["fast (a not run)"])


class SpecPurposeTests(unittest.TestCase):
    def test_protected_specs_cannot_be_logic_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "qa-axe.spec.ts").write_text("test('x', { tag: '@logic' }, () => {});\n")
            (root / "cart.spec.ts").write_text("test('x', { tag: '@logic' }, () => {});\n")
            violations = gate.protected_spec_violations(root)
            self.assertEqual(len(violations), 1)
            self.assertIn("qa-axe.spec.ts", violations[0])


class SettingsTests(unittest.TestCase):
    def test_settings_parse(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "baukit.toml"
            path.write_text('[quality.affected]\n"web/**" = ["web"]\n')
            self.assertEqual(gate.load_settings(path)["quality"]["affected"], {"web/**": ["web"]})


if __name__ == "__main__":
    unittest.main()
