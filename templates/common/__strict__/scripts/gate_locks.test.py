import fcntl
import importlib.util
import os
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
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


locks = load("gate_locks")
gate = load("gate")

POOL_WORKER = textwrap.dedent("""
    import sys, time
    sys.path.insert(0, sys.argv[1])
    import gate_locks
    from pathlib import Path
    pool = gate_locks.CorePool(Path(sys.argv[2]), int(sys.argv[3]))
    events = sys.argv[4]
    def log(sign, cores):
        with open(events, "a") as out:
            out.write(f"{sign}{cores}\\n")
    for _ in range(6):
        cores = pool.acquire(2, 1)
        log("+", cores)
        time.sleep(0.05)
        log("-", cores)
        pool.release(cores)
""")


def excl(*names):
    return [locks.Need(name) for name in names]


def shared(name):
    return locks.Need(name, shared=True)


def is_locked(path):
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return True
    finally:
        os.close(descriptor)
    return False


class ResourceTests(unittest.TestCase):
    def test_defaults_name_the_emulator_and_the_compose_network(self):
        resources = locks.resource_map({})
        self.assertEqual(
            locks.resources_for({"android-e2e", "mobile"}, resources),
            excl("android", "docker-network"),
        )
        self.assertEqual(locks.resources_for({"playwright", "web"}, resources), [])
        self.assertEqual(locks.resources_for({"web-unit", "web"}, resources), [])

    def test_settings_extend_the_resources(self):
        settings = {
            "quality": {
                "resources": {"android": ["my-emulator"], "docker-network": ["compose"]},
                "shared_resources": {"docker-network": ["host-browser"], "gpu": ["host-browser"]},
            }
        }
        resources = locks.resource_map(settings)
        self.assertEqual(locks.resources_for({"my-emulator"}, resources), excl("android"))
        self.assertEqual(locks.resources_for({"x", "compose"}, resources), excl("docker-network"))
        self.assertEqual(
            locks.resources_for({"host-browser"}, resources),
            [shared("docker-network"), shared("gpu")],
        )
        self.assertEqual(
            locks.resources_for({"host-browser", "compose"}, resources),
            [locks.Need("docker-network"), shared("gpu")],
        )

    def test_unknown_members_are_reported(self):
        settings = {
            "quality": {
                "resources": {"android": ["nope", "web"]},
                "shared_resources": {"gpu": ["gone"]},
            }
        }
        self.assertEqual(locks.unknown_resource_members(settings, {"web"}), ["gone", "nope"])

    def test_locks_are_held_only_while_the_step_runs_in_order(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            android, browser = Path(directory) / "android.lock", Path(directory) / "browser.lock"
            holder.directory.mkdir(exist_ok=True)
            with holder.hold(excl("android", "browser")) as waits:
                self.assertEqual([wait.resource for wait in waits], ["android", "browser"])
                self.assertTrue(is_locked(android))
                self.assertTrue(is_locked(browser))
            self.assertFalse(is_locked(android))
            self.assertFalse(is_locked(browser))

    def test_a_second_holder_waits_for_the_first(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            order = []
            with holder.hold(excl("android")):

                def contend():
                    with holder.hold(excl("android")):
                        order.append("got")

                waiter = threading.Thread(target=contend)
                waiter.start()
                waiter.join(0.2)
                self.assertEqual(order, [])
            waiter.join(2)
            self.assertEqual(order, ["got"])

    def test_a_two_lock_step_is_not_overtaken_by_later_single_lock_steps(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            order = []

            def take(resources, label, seconds):
                with holder.hold(resources):
                    order.append(label)
                    time.sleep(seconds)

            with holder.hold(excl("browser")):
                both = threading.Thread(
                    target=take, args=(excl("android", "browser"), "both", 0.05)
                )
                both.start()
                while len(list(holder.queue.glob("*.ticket"))) < 1:
                    time.sleep(0.01)
                singles = [
                    threading.Thread(target=take, args=(excl("browser"), f"single{n}", 0.05))
                    for n in range(6)
                ]
                for single in singles:
                    single.start()
                    time.sleep(0.02)
            for thread in [both, *singles]:
                thread.join(30)
            self.assertEqual(order[0], "both")
            self.assertEqual(len(order), 7)

    def test_a_two_lock_step_gets_its_locks_while_single_lock_steps_keep_arriving(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            stop = threading.Event()
            workers = []

            def stream():
                while not stop.is_set():
                    with holder.hold(excl("browser")):
                        time.sleep(0.03)

            for _ in range(3):
                worker = threading.Thread(target=stream)
                worker.start()
                workers.append(worker)
            time.sleep(0.2)
            started = time.monotonic()
            with holder.hold(excl("android", "browser")):
                waited = time.monotonic() - started
            stop.set()
            for worker in workers:
                worker.join(10)
            self.assertLess(waited, 2)

    def test_a_dead_waiter_does_not_block_later_steps(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            code = (
                "import sys; sys.path.insert(0, sys.argv[1]); import gate_locks;"
                "from pathlib import Path;"
                "gate_locks.ResourceLocks(Path(sys.argv[2])).enqueue("
                "[gate_locks.Need('android'), gate_locks.Need('browser')])"
            )
            subprocess.run([sys.executable, "-c", code, str(SCRIPTS), directory], check=True)
            self.assertEqual(len(list(holder.queue.glob("*.ticket"))), 1)
            with holder.hold(excl("browser")):
                pass
            self.assertEqual(list(holder.queue.glob("*.ticket")), [])

    def test_disjoint_resources_do_not_wait_for_each_other(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            with holder.hold(excl("android")):
                started = time.monotonic()
                with holder.hold(excl("docker-network")):
                    pass
                self.assertLess(time.monotonic() - started, 1)

    def test_shared_holders_run_together_and_exclude_an_exclusive_holder(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            with holder.hold([shared("docker-network")]):
                started = time.monotonic()
                with holder.hold([shared("docker-network")]):
                    self.assertLess(time.monotonic() - started, 1)
                got = []

                def exclusive():
                    with holder.hold(excl("docker-network")):
                        got.append(1)

                waiter = threading.Thread(target=exclusive)
                waiter.start()
                waiter.join(0.3)
                self.assertEqual(got, [])
            waiter.join(10)
            self.assertEqual(got, [1])

    def test_an_exclusive_waiter_gets_through_continuous_shared_arrivals(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            stop = threading.Event()
            workers = []

            def stream():
                while not stop.is_set():
                    with holder.hold([shared("docker-network")]):
                        time.sleep(0.03)

            for _ in range(4):
                worker = threading.Thread(target=stream)
                worker.start()
                workers.append(worker)
            time.sleep(0.2)
            started = time.monotonic()
            with holder.hold(excl("docker-network")):
                waited = time.monotonic() - started
                self.assertTrue(is_locked(Path(directory) / "docker-network.lock"))
            stop.set()
            for worker in workers:
                worker.join(10)
            self.assertLess(waited, 2)

    def test_shared_steps_queued_behind_an_exclusive_waiter_wait_for_it(self):
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory))
            order = []

            def take(need, label):
                with holder.hold([need]):
                    order.append(label)
                    time.sleep(0.05)

            with holder.hold([shared("docker-network")]):
                exclusive = threading.Thread(
                    target=take, args=(locks.Need("docker-network"), "exclusive")
                )
                exclusive.start()
                while not list(holder.queue.glob("*.ticket")):
                    time.sleep(0.01)
                later = threading.Thread(target=take, args=(shared("docker-network"), "later"))
                later.start()
                later.join(0.3)
                self.assertEqual(order, [])
            exclusive.join(10)
            later.join(10)
            self.assertEqual(order, ["exclusive", "later"])

    def test_wait_text_names_each_lock(self):
        text = locks.describe_waits([locks.LockWait("android", 3), locks.LockWait("browser", 0)])
        self.assertEqual(text, "waited 3 s for android.lock, waited 0 s for browser.lock")


class CorePoolTests(unittest.TestCase):
    def test_slots_are_taken_and_released(self):
        with tempfile.TemporaryDirectory() as directory:
            pool = locks.CorePool(Path(directory), 3)
            self.assertEqual(pool.acquire(2, 1), 2)
            self.assertEqual(pool.available(), 1)
            self.assertEqual(pool.acquire(5, 1), 1)
            self.assertEqual(pool.available(), 0)
            pool.release(2)
            self.assertEqual(pool.available(), 2)
            pool.release(1)
            self.assertEqual(pool.available(), 3)

    def test_a_step_waits_for_its_minimum(self):
        with tempfile.TemporaryDirectory() as directory:
            pool = locks.CorePool(Path(directory), 4)
            self.assertEqual(pool.acquire(3, 3), 3)
            granted = []
            waiter = threading.Thread(target=lambda: granted.append(pool.acquire(4, 2)))
            waiter.start()
            waiter.join(0.4)
            self.assertEqual(granted, [])
            pool.release(3)
            waiter.join(3)
            self.assertEqual(granted, [4])

    def test_core_budget_draws_from_the_pool(self):
        with tempfile.TemporaryDirectory() as directory:
            budget = gate.CoreBudget(4, None, 1024, 2, locks.CorePool(Path(directory), 4))
            other = locks.CorePool(Path(directory), 4)
            self.assertEqual(budget.acquire(3), 3)
            self.assertEqual(other.available(), 1)
            self.assertEqual(budget.available(), 1)
            budget.release(3)
            self.assertEqual(other.available(), 4)

    def test_concurrent_processes_never_exceed_the_pool(self):
        size = 4
        with tempfile.TemporaryDirectory() as directory:
            events = Path(directory) / "events"
            pool_directory = Path(directory) / "pool"
            workers = [
                subprocess.Popen(
                    [
                        sys.executable,
                        "-c",
                        POOL_WORKER,
                        str(SCRIPTS),
                        str(pool_directory),
                        str(size),
                        str(events),
                    ]
                )
                for _ in range(3)
            ]
            for worker in workers:
                self.assertEqual(worker.wait(60), 0)
            running = peak = 0
            for line in events.read_text().split():
                running += int(line)
                peak = max(peak, running)
            self.assertEqual(running, 0)
            self.assertGreater(peak, 2)
            self.assertLessEqual(peak, size)

    def test_gates_in_one_pool_split_the_cores(self):
        with tempfile.TemporaryDirectory() as directory:
            first = gate.CoreBudget(12, None, 1024, 2, locks.CorePool(Path(directory), 12))
            self.assertEqual(first.share(1), 12)
            second = gate.CoreBudget(12, None, 1024, 2, locks.CorePool(Path(directory), 12))
            self.assertEqual(first.share(1), 6)
            self.assertEqual(second.share(2), 3)
            code = (
                "import sys; sys.path.insert(0, sys.argv[1]); import gate_locks;"
                "from pathlib import Path;"
                "gate_locks.CorePool(Path(sys.argv[2]), 12)"
            )
            subprocess.run([sys.executable, "-c", code, str(SCRIPTS), directory], check=True)
            self.assertEqual(first.share(1), 6)

    def test_a_dead_process_frees_its_slots(self):
        with tempfile.TemporaryDirectory() as directory:
            code = (
                "import sys; sys.path.insert(0, sys.argv[1]); import gate_locks;"
                "from pathlib import Path;"
                "gate_locks.CorePool(Path(sys.argv[2]), 2).acquire(2, 2)"
            )
            subprocess.run([sys.executable, "-c", code, str(SCRIPTS), directory], check=True)
            self.assertEqual(locks.CorePool(Path(directory), 2).available(), 2)

    def test_children_inherit_no_lock_or_slot_descriptors(self):
        listing = (
            "import os;"
            "print('\\n'.join(os.readlink(f'/proc/self/fd/{n}') for n in os.listdir('/proc/self/fd') "
            "if os.path.exists(f'/proc/self/fd/{n}')))"
        )
        with tempfile.TemporaryDirectory() as directory:
            holder = locks.ResourceLocks(Path(directory) / "locks")
            pool = locks.CorePool(Path(directory) / "pool", 2)
            pool.acquire(2, 2)
            with holder.hold(excl("android", "browser")):
                child = subprocess.run(
                    [sys.executable, "-c", listing], capture_output=True, text=True, check=True
                )
            self.assertNotIn(directory, child.stdout)


FAKE_STEPS = [
    "lint fast - always -",
    "unit web - selected -",
    "playwright web unit selected -",
    "android-e2e mobile - selected -",
]


class RunnerLockTests(unittest.TestCase):
    def run_gate(self, failing, *, extra_env=None, settings_text="", steps=FAKE_STEPS):
        directory = tempfile.TemporaryDirectory()
        root = Path(directory.name)
        script = root / "steps.sh"
        trace = root / "trace"
        script.write_text(
            textwrap.dedent(f"""\
            if [ "$1" = --list-steps ]; then
              printf '%s\\n' {" ".join(repr(line) for line in steps)}; exit 0
            fi
            printf '%s start android=%s network=%s\\n' "$2" \\
              "$(python3 -c 'import fcntl,os,sys
d=os.open(sys.argv[1],os.O_RDWR|os.O_CREAT)
try: fcntl.flock(d,fcntl.LOCK_EX|fcntl.LOCK_NB); print(0)
except BlockingIOError: print(1)' "{root}/locks/android.lock")" \\
              "$(python3 -c 'import fcntl,os,sys
d=os.open(sys.argv[1],os.O_RDWR|os.O_CREAT)
try: fcntl.flock(d,fcntl.LOCK_EX|fcntl.LOCK_NB); print(0)
except BlockingIOError: print(1)' "{root}/locks/docker-network.lock")" >> "{trace}"
            case " {" ".join(failing)} " in *" $2 "*) exit 3 ;; esac
        """)
        )
        settings = root / "settings.toml"
        settings.write_text(settings_text)
        (root / "locks").mkdir()
        environment = {
            "PATH": os.environ["PATH"],
            "PYTHONPATH": os.environ.get("PYTHONPATH", ""),
            "BAUKIT_GATE_CORES": "4",
            "BAUKIT_GATE_LOCK_DIR": str(root / "locks"),
            **(extra_env or {}),
        }
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "gate.py"),
                "run",
                "--script",
                f"sh {script}",
                "--settings",
                str(settings),
                "--log-dir",
                str(root / "logs"),
            ],
            capture_output=True,
            text=True,
            check=False,
            env=environment,
        )
        lines = trace.read_text().splitlines() if trace.exists() else []
        directory.cleanup()
        return result, lines

    def held_by(self, step_line):
        result, trace = self.run_gate([], steps=[step_line])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return trace[0].split()[2:], result

    def test_locks_are_held_only_around_resource_steps(self):
        self.assertEqual(self.held_by("lint fast - always -")[0], ["android=0", "network=0"])
        self.assertEqual(self.held_by("unit web - selected -")[0], ["android=0", "network=0"])
        self.assertEqual(self.held_by("playwright web - selected -")[0], ["android=0", "network=0"])
        held, result = self.held_by("android-e2e mobile - selected -")
        self.assertEqual(held, ["android=1", "network=1"])
        self.assertIn(
            "waited 0 s for android.lock, waited 0 s for docker-network.lock", result.stdout
        )

    def test_heavy_steps_do_not_start_after_a_fast_lane_failure(self):
        text = '[quality.shared_resources]\ndocker-network = ["playwright"]\n'
        result, trace = self.run_gate(["lint"], settings_text=text)
        self.assertEqual(result.returncode, 1)
        started = {line.split()[0] for line in trace}
        self.assertNotIn("android-e2e", started)
        self.assertNotIn("playwright", started)
        self.assertIn("unit", started)
        self.assertIn("playwright not run (fast lane failed)", result.stdout)
        self.assertIn("android-e2e not run (fast lane failed)", result.stdout)

    def test_without_a_lock_dir_heavy_steps_do_not_wait_for_the_fast_lane(self):
        result, trace = self.run_gate(["lint"], extra_env={"BAUKIT_GATE_LOCK_DIR": ""})
        self.assertEqual(result.returncode, 1)
        self.assertIn("android-e2e", {line.split()[0] for line in trace})
        self.assertNotIn("fast lane failed", result.stdout)

    def test_a_product_resource_takes_the_lock(self):
        result, trace = self.run_gate([], settings_text='[quality.resources]\nandroid = ["unit"]\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        held = {line.split()[0]: line.split()[2:] for line in trace}
        self.assertEqual(held["unit"], ["android=1", "network=0"])

    def test_a_product_shared_resource_takes_a_shared_lock(self):
        text = '[quality.shared_resources]\ndocker-network = ["playwright"]\n'
        result, trace = self.run_gate([], settings_text=text)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        held = {line.split()[0]: line.split()[2:] for line in trace}
        self.assertEqual(held["playwright"], ["android=0", "network=1"])
        self.assertIn("playwright takes docker-network shared", result.stdout)

    def test_unknown_resource_members_fail(self):
        result, _ = self.run_gate([], settings_text='[quality.resources]\nandroid = ["nope"]\n')
        self.assertEqual(result.returncode, 2)
        self.assertIn(
            "quality.resources and quality.shared_resources name unknown lanes or steps: nope",
            result.stderr,
        )

    def test_pool_reports_free_cores(self):
        with tempfile.TemporaryDirectory() as pool:
            result, _ = self.run_gate([], extra_env={"BAUKIT_GATE_CORE_POOL": pool})
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertRegex(result.stdout, r"\d of 4 pool cores free")
            self.assertTrue((Path(pool) / "slot-0.lock").exists())
            self.assertFalse((Path(pool) / "slot-4.lock").exists())

    def test_a_resource_step_waits_for_a_held_lock_before_taking_cores(self):
        with tempfile.TemporaryDirectory() as pool:
            lock_dir = Path(pool) / "locks"
            lock_dir.mkdir()
            holder = locks.ResourceLocks(lock_dir)
            with holder.hold(excl("docker-network")):
                started = time.monotonic()
                timer = threading.Timer(1.5, lambda: None)
                timer.start()
                outcome = []
                worker = threading.Thread(
                    target=lambda: outcome.append(
                        self.run_gate(
                            [],
                            extra_env={
                                "BAUKIT_GATE_LOCK_DIR": str(lock_dir),
                                "BAUKIT_GATE_CORE_POOL": str(Path(pool) / "cores"),
                            },
                            steps=["android-e2e mobile - selected -"],
                        )
                    )
                )
                worker.start()
                timer.join()
                self.assertTrue(worker.is_alive())
                self.assertEqual(locks.CorePool(Path(pool) / "cores", 4).available(), 4)
            worker.join(30)
            result, _ = outcome[0]
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertGreaterEqual(time.monotonic() - started, 1.5)
            self.assertRegex(result.stdout, r"waited [1-9]\d* s for docker-network.lock")


if __name__ == "__main__":
    unittest.main()
