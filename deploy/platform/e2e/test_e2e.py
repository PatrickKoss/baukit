import importlib.util
import json
import os
import re
import sys
import tempfile
import time
import unittest
from datetime import datetime
from pathlib import Path

E2E = Path(__file__).resolve().parent
PLATFORM = E2E.parent
REPO = PLATFORM.parent.parent
RECONCILIATION = E2E / "overlay/reconciliation.yaml"
NOT_BASES = {"components", "e2e", "lib", "lint", "overlays", "schemas"}
# tailscale-operator needs a real tailnet OAuth client, which e2e does not have. emulator-slots
# needs /dev/kvm on the node, which CI runners do not promise.
LEFT_OUT = {
    "emulator-slots",
    "hcloud",
    "posthog",
    "redis",
    "tailscale-operator",
    "tailscale-proxy-class",
}
NODE_CIDR_PLACEHOLDER = "192.0.2.0/32"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


process_runner = load_module("process_runner", REPO / "scripts/process_runner.py")
helm_yaml = load_module("helm_yaml", PLATFORM / "lint/helm_yaml.py")


def kustomization(name, depends_on=()):
    spec = {"dependsOn": [{"name": dependency} for dependency in depends_on]}
    return {
        "apiVersion": "kustomize.toolkit.fluxcd.io/v1",
        "kind": "Kustomization",
        "metadata": {"name": name},
        "spec": spec,
    }


def run_without(document, *names):
    return process_runner.run_process(
        [sys.executable, str(E2E / "without.py"), *names],
        input="\n".join(json.dumps(item) for item in document["items"]),
        capture_output=True,
        text=True,
        check=False,
    )


def referenced_bases():
    paths = re.findall(r"^  path: \./deploy/platform/(\S+)$", RECONCILIATION.read_text(), re.M)
    pending = [PLATFORM / path for path in paths]
    seen = set()
    while pending:
        directory = pending.pop().resolve()
        if directory in seen:
            continue
        seen.add(directory)
        text = (directory / "kustomization.yaml").read_text()
        for reference in re.findall(r"^\s+- (\S+)$", text, re.M):
            target = directory / reference
            if (target / "kustomization.yaml").is_file():
                pending.append(target)
    return {
        directory.relative_to(PLATFORM).parts[0]
        for directory in seen
        if directory.parent == PLATFORM and directory.name not in NOT_BASES
    }


class WithoutTests(unittest.TestCase):
    def test_drops_the_kustomization_and_its_dependency_edges(self):
        document = {
            "kind": "List",
            "items": [kustomization("e2e-a"), kustomization("e2e-b", ["e2e-a", "e2e-c"])],
        }
        result = run_without(document, "e2e-a")
        self.assertEqual(result.returncode, 0, result.stderr)
        items = json.loads(result.stdout)["items"]
        self.assertEqual([item["metadata"]["name"] for item in items], ["e2e-b"])
        self.assertEqual(items[0]["spec"]["dependsOn"], [{"name": "e2e-c"}])

    def test_keeps_everything_without_names(self):
        document = {"apiVersion": "v1", "kind": "List", "items": [kustomization("e2e-a")]}
        result = run_without(document)
        self.assertEqual(json.loads(result.stdout), document)

    def test_unknown_name_exits_2(self):
        result = run_without({"kind": "List", "items": [kustomization("e2e-a")]}, "e2e-x")
        self.assertEqual(result.returncode, 2)
        self.assertIn("e2e-x", result.stderr)


class OverlayTests(unittest.TestCase):
    def test_composes_every_base_except_the_left_out_ones(self):
        bases = {
            path.parent.name
            for path in PLATFORM.glob("*/kustomization.yaml")
            if path.parent.name not in NOT_BASES
        }
        self.assertEqual(referenced_bases(), bases - LEFT_OUT)

    def test_no_kustomization_keeps_the_node_cidr_placeholder(self):
        paths = re.findall(r"^  path: (\./deploy/platform/\S+)$", RECONCILIATION.read_text(), re.M)
        for path in sorted(set(paths)):
            with self.subTest(path=path):
                result = process_runner.run_process(
                    ["kustomize", "build", path], capture_output=True, text=True, cwd=REPO
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertNotIn(NODE_CIDR_PLACEHOLDER, result.stdout)

    def test_a_base_with_a_local_component_is_composed_through_it(self):
        local = PLATFORM / "overlays/local/components"
        paths = set(re.findall(r"^  path: \./(\S+)$", RECONCILIATION.read_text(), re.M))
        components = [path.parent for path in local.glob("*/kustomization.yaml")]
        for component in sorted(c for c in components if c.name not in LEFT_OUT):
            with self.subTest(base=component.name):
                own = E2E / "overlay/components" / component.name / "kustomization.yaml"
                via_local = str(component.relative_to(REPO)) in paths
                reference = f"../../../../overlays/local/components/{component.name}\n"
                via_own = own.is_file() and reference in own.read_text()
                self.assertTrue(via_local or via_own, f"e2e composes {component.name} without it")

    def test_tenant_waits_for_the_policies(self):
        text = RECONCILIATION.read_text()
        tenant = next(doc for doc in text.split("\n---\n") if "name: e2e-tenant\n" in doc)
        self.assertIn("- name: e2e-kyverno-policies", tenant)


class RunnerArgumentTests(unittest.TestCase):
    def run_runner(self, *arguments):
        environment = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
        return process_runner.run_process(
            [str(E2E / "run.sh"), *arguments],
            capture_output=True,
            text=True,
            check=False,
            env=environment,
            cwd=REPO,
        )

    def test_unknown_flag_exits_2(self):
        result = self.run_runner("--bogus")
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)

    def test_bad_cluster_name_exits_2(self):
        result = self.run_runner("--cluster-name", "Bad_Name")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--cluster-name", result.stderr)

    def test_unknown_without_exits_2(self):
        result = self.run_runner("--without", "no-such-kustomization")
        self.assertEqual(result.returncode, 2)
        self.assertIn("e2e-no-such-kustomization", result.stderr)

    def test_missing_suite_exits_2(self):
        result = self.run_runner("--suite", "no-such-suite")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no chainsaw-test.yaml", result.stderr)

    def parsed_suites(self, *arguments):
        script = (
            f'source "{E2E / "run.sh"}"\n'
            f"parse_args {' '.join(arguments)}\n"
            'printf "%s\\n" "${test_dirs[@]}"'
        )
        result = process_runner.run_process(["bash", "-c", script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return [Path(line).name for line in result.stdout.splitlines()]

    def test_suite_repeats_and_replaces_the_default(self):
        self.assertEqual(self.parsed_suites(), ["tests"])
        self.assertEqual(self.parsed_suites("--suite", "netpol"), ["netpol"])
        self.assertEqual(
            self.parsed_suites("--suite", "netpol", "--suite", "ingress"), ["netpol", "ingress"]
        )


class HeavySlotTests(unittest.TestCase):
    def start_wait(self, slot_dir):
        environment = {
            **os.environ,
            "BAUKIT_HEAVY_SLOT_DIR": slot_dir,
            "BAUKIT_HEAVY_SLOT_POLL_SECONDS": "0.1",
            "RUN_SH": str(E2E / "run.sh"),
        }
        return process_runner.start_process(
            ["bash", "-c", 'source "$RUN_SH" && wait_for_heavy_slot && echo proceeded'],
            stdout=process_runner.subprocess.PIPE,
            text=True,
            env=environment,
        )

    def test_blocks_while_both_slots_exist_and_proceeds_when_one_frees(self):
        with tempfile.TemporaryDirectory() as slot_dir:
            for slot in ("slot-1", "slot-2"):
                Path(slot_dir, slot).mkdir()
            process = self.start_wait(slot_dir)
            time.sleep(1)
            self.assertIsNone(process.poll(), "the wait returned while both slots were taken")
            Path(slot_dir, "slot-2").rmdir()
            output, _ = process.communicate(timeout=10)
        self.assertEqual(process.returncode, 0)
        self.assertIn("Waiting for a free heavy-run slot", output)
        self.assertTrue(output.endswith("proceeded\n"))

    def test_no_slot_wait_without_a_slot_dir(self):
        environment = {k: v for k, v in os.environ.items() if k != "BAUKIT_HEAVY_SLOT_DIR"}
        environment["RUN_SH"] = str(E2E / "run.sh")
        result = process_runner.run_process(
            ["bash", "-c", 'source "$RUN_SH" && wait_for_heavy_slot && echo proceeded'],
            capture_output=True,
            text=True,
            env=environment,
        )
        self.assertEqual(result.stdout, "proceeded\n")

    def test_lock_defaults_to_the_user_cache(self):
        environment = {k: v for k, v in os.environ.items() if k != "BAUKIT_E2E_CLUSTER_LOCK"}
        environment.update(XDG_CACHE_HOME="/xdg", RUN_SH=str(E2E / "run.sh"))
        result = process_runner.run_process(
            ["bash", "-c", 'source "$RUN_SH" && printf %s "$cluster_lock"'],
            capture_output=True,
            text=True,
            env=environment,
        )
        self.assertEqual(result.stdout, "/xdg/baukit/platform-e2e.lock")

    def test_proceeds_at_once_with_one_slot_free(self):
        with tempfile.TemporaryDirectory() as slot_dir:
            Path(slot_dir, "slot-1").mkdir()
            output, _ = self.start_wait(slot_dir).communicate(timeout=10)
        self.assertEqual(output, "proceeded\n")


class DatastoreTests(unittest.TestCase):
    def test_cluster_uses_the_local_etcd_helper(self):
        text = (E2E / "run.sh").read_text()
        self.assertIn("mapfile -t etcd_args < <(platform_local_etcd_args)", text)
        self.assertIn('"${etcd_args[@]}" \\', text)
        self.assertIn("mapfile -t kubelet_args < <(platform_local_kubelet_args)", text)
        self.assertIn('"${kubelet_args[@]}" \\', text)


class NodeRegistrationTests(unittest.TestCase):
    def wait(self, empty_calls):
        with tempfile.TemporaryDirectory() as tmp:
            count = Path(tmp) / "count"
            count.write_text("0")
            stubs = {
                "kubectl": f'n=$(cat "{count}"); echo $((n + 1)) >"{count}"; '
                f'[ "$n" -ge {empty_calls} ] && echo node/server-0; exit 0',
                "docker": 'echo "docker $*"',
            }
            for name, body in stubs.items():
                stub = Path(tmp) / name
                stub.write_text(f"#!/bin/sh\n{body}\n")
                stub.chmod(0o755)
            environment = {
                **os.environ,
                "PATH": f"{tmp}:{os.environ['PATH']}",
                "BAUKIT_E2E_NODE_POLL_SECONDS": "0",
                "RUN_SH": str(E2E / "run.sh"),
            }
            result = process_runner.run_process(
                ["bash", "-c", 'source "$RUN_SH" && wait_for_node_registration e2e'],
                capture_output=True,
                text=True,
                env=environment,
            )
            return result, int(count.read_text())

    def test_waits_until_a_node_registers(self):
        result, calls = self.wait(empty_calls=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, 4)

    def test_gives_up_and_prints_the_server_log(self):
        result, calls = self.wait(empty_calls=1000)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, 60)
        self.assertIn("no node registered in cluster e2e", result.stderr)
        self.assertIn("docker logs --tail 30 k3d-e2e-server-0", result.stderr)


def run_with_stubs(stubs, call, **extra):
    with tempfile.TemporaryDirectory() as tmp:
        for name, body in stubs.items():
            stub = Path(tmp) / name
            stub.write_text(f"#!/bin/sh\n{body}\n")
            stub.chmod(0o755)
        environment = {
            **os.environ,
            "PATH": f"{tmp}:{os.environ['PATH']}",
            "RUN_SH": str(E2E / "run.sh"),
            "TMP": tmp,
            **extra,
        }
        result = process_runner.run_process(
            ["bash", "-c", f'set -euo pipefail; source "$RUN_SH"; {call}'],
            capture_output=True,
            text=True,
            env=environment,
        )
        files = {
            path.relative_to(tmp).as_posix(): path.read_text()
            for path in Path(tmp).rglob("*")
            if path.is_file() and path.name not in stubs
        }
        return result, files


UNAVAILABLE = 'echo "Error from server (ServiceUnavailable)" >&2; exit 1'


class UnavailableApiServerTests(unittest.TestCase):
    def test_failed_kubectl_reads_as_not_ready(self):
        for function in ("stalled_releases", "failed_kustomizations"):
            with self.subTest(function=function):
                result, _ = run_with_stubs({"kubectl": UNAVAILABLE}, f"{function}; echo polled")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "polled\n")
                self.assertNotIn("JSONDecodeError", result.stderr)

    def test_empty_output_reads_as_not_ready(self):
        result, _ = run_with_stubs({"kubectl": "exit 0"}, "failed_kustomizations; echo polled")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "polled\n")

    def test_artifacts_keep_the_server_log_and_restart_count(self):
        stubs = {
            "kubectl": UNAVAILABLE,
            "flux": "exit 1",
            "docker": 'case "$1" in logs) echo "2026-11-15T19:31:00Z k3s restarted";; '
            "inspect) echo 2;; esac",
        }
        call = (
            'artifacts_dir="$TMP/artifacts"; cluster_name=e2e; '
            'mkdir -p "$artifacts_dir"; collect_artifacts'
        )
        result, files = run_with_stubs(stubs, call)
        self.assertEqual(result.returncode, 0, result.stderr)
        log = files["artifacts/logs/k3d-e2e-server-0.log"]
        self.assertIn("k3s restarted", log)
        self.assertEqual(files["artifacts/logs/k3d-e2e-server-0.restart-count"], "2\n")


class RuntimeIdentityTests(unittest.TestCase):
    def run_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            calls = Path(tmp) / "calls"
            kubectl = Path(tmp) / "kubectl"
            kubectl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >>"$CALLS"\n')
            kubectl.chmod(0o755)
            helper = PLATFORM / "runtime-identity.sh"
            script = f'source "{helper}"; platform_runtime_identity e2e p a s u g'
            env = {**os.environ, "PATH": f"{tmp}:{os.environ['PATH']}", "CALLS": str(calls)}
            result = process_runner.run_process(
                ["bash", "-c", script], env=env, capture_output=True, text=True
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return calls.read_text().splitlines()

    def test_cluster_config_map_lands_where_alloy_runs(self):
        calls = self.run_identity()
        self.assertIn(
            "-n observability-agents create configmap observability-cluster "
            "--from-literal=environment=e2e --dry-run=client -o yaml",
            calls,
        )

    def test_secrets_land_in_their_consumer_namespaces(self):
        created = {
            tuple(call.split()[1:2] + call.split()[5:6])
            for call in self.run_identity()
            if " create secret generic " in call
        }
        self.assertEqual(
            created,
            {
                ("postgres", "product-db-credentials"),
                ("postgres", "postgres-backup-credentials"),
                ("postgres", "rustfs-root"),
                ("observability", "rustfs-root"),
                ("observability", "grafana-admin"),
            },
        )

    def test_pre_created_namespaces_carry_their_base_psa_labels(self):
        manifests = {
            doc["metadata"]["name"]: doc["metadata"]["labels"]
            for path in PLATFORM.glob("*/namespace.yaml")
            for doc in helm_yaml.parse_documents(helm_yaml.split_documents(path.read_text()))
            if doc.get("kind") == "Namespace"
        }
        names = self.source_identity('printf "%s\\n" "${platform_runtime_namespaces[@]}"').split()
        for name in [*names, "posthog"]:
            with self.subTest(namespace=name):
                applied = helm_yaml.parse_documents(
                    [self.source_identity(f"platform_namespace_manifest {name}")]
                )[0]
                labels = applied["metadata"]["labels"]
                psa = {k: v for k, v in labels.items() if k.startswith("pod-security.")}
                self.assertEqual(len(psa), 4)
                self.assertEqual(labels, manifests[name])

    def test_unknown_namespace_fails(self):
        helper = PLATFORM / "runtime-identity.sh"
        result = process_runner.run_process(
            ["bash", "-c", f'source "{helper}"; platform_ensure_namespace nope'],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Namespace nope", result.stderr)

    def source_identity(self, call):
        helper = PLATFORM / "runtime-identity.sh"
        result = process_runner.run_process(
            ["bash", "-c", f'source "{helper}"; {call}'], capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_both_entry_points_share_the_helper(self):
        for script in (PLATFORM / "platform-up.sh", E2E / "run.sh"):
            text = script.read_text()
            self.assertIn("platform_runtime_identity ", text, script)
            self.assertNotIn("create configmap observability-cluster", text, script)


class KubeletImageGcTests(unittest.TestCase):
    def test_e2e_cluster_never_garbage_collects_imported_images(self):
        text = (E2E / "run.sh").read_text()
        arg = "--kubelet-arg=image-gc-high-threshold=100@server:0"
        self.assertIn(f"kubelet_image_gc_arg='{arg}'", text)
        self.assertIn('--k3s-arg "$kubelet_image_gc_arg"', text)


def functions_of(*names):
    ranges = "; ".join(f"/^{name}() {{/,/^}}/p" for name in names)
    return f'source <(sed -n "{ranges}" "{E2E / "run.sh"}")'


class StalledReleaseTests(unittest.TestCase):
    def test_only_stalled_releases_are_printed(self):
        releases = {
            "items": [
                {
                    "metadata": {"namespace": "falco", "name": "falco"},
                    "status": {"conditions": [{"type": "Stalled", "status": "True"}]},
                },
                {
                    "metadata": {"namespace": "kyverno", "name": "kyverno"},
                    "status": {"conditions": [{"type": "Ready", "status": "True"}]},
                },
                {"metadata": {"namespace": "new", "name": "new"}},
            ]
        }
        with tempfile.TemporaryDirectory() as tmp:
            listing = Path(tmp) / "releases.json"
            listing.write_text(json.dumps(releases))
            script = (
                f'kubectl() {{ cat "{listing}"; }}\n'
                f"{functions_of('flux_items_json', 'stalled_releases')}\n"
                "stalled_releases"
            )
            result = process_runner.run_process(
                ["bash", "-c", script], capture_output=True, text=True
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "falco/falco\n")


class FailedKustomizationTests(unittest.TestCase):
    def run_function(self, function, setup, call):
        script = f"{setup}\n{functions_of('flux_items_json', function)}\n{call}"
        result = process_runner.run_process(["bash", "-c", script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_only_non_transient_failures_are_printed(self):
        def item(name, reason):
            condition = {"type": "Ready", "status": "False", "reason": reason}
            return {
                "metadata": {"namespace": "flux-system", "name": name},
                "status": {"conditions": [condition]},
            }

        listing = {
            "items": [
                item("e2e-tenant", "ReconciliationFailed"),
                item("e2e-broken", "BuildFailed"),
                item("e2e-redis", "DependencyNotReady"),
                item("e2e-postgres", "Progressing"),
            ]
        }
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ks.json"
            path.write_text(json.dumps(listing))
            output = self.run_function(
                "failed_kustomizations", f'kubectl() {{ cat "{path}"; }}', "failed_kustomizations"
            )
        self.assertEqual(output, "flux-system/e2e-tenant\nflux-system/e2e-broken\n")

    def test_a_failure_counts_only_after_three_polls_in_a_row(self):
        with tempfile.TemporaryDirectory() as tmp:
            counts = Path(tmp) / "counts"
            poll = f'persistent_failures "{counts}"'
            calls = "\n".join(
                [
                    f'{poll} "a"; echo "|"',
                    f'{poll} "a"; echo "|"',
                    f'{poll} ""; echo "|"',
                    f'{poll} "a"; echo "|"',
                    f'{poll} "a"; echo "|"',
                    f'{poll} "a"; echo "|"',
                ]
            )
            output = self.run_function("persistent_failures", "failed_poll_limit=3", calls)
        self.assertEqual(output.split("|\n"), ["", "", "", "", "", "a\n", ""])


class IngressSuiteTests(unittest.TestCase):
    def test_echo_keeps_the_chart_default_ingress_controller_peer(self):
        values = (E2E / "tests/ingress/values.yaml").read_text()
        self.assertNotRegex(values, r"(?m)^\s*ingressController:")
        self.assertRegex(values, r"(?m)^ingress:\n  enabled: true$")


class SubnetTests(unittest.TestCase):
    """free_subnet and create_k3d_cluster with stubbed docker and k3d binaries."""

    def run_runner_function(self, call, subnets, k3d_failures="", extra_env=None):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp)
            calls = bin_dir / "calls"
            failures = bin_dir / "failures"
            failures.write_text(k3d_failures)
            (bin_dir / "docker").write_text(
                "#!/bin/sh\n"
                'case "$1 $2" in\n'
                '  "network ls") echo net1 ;;\n'
                '  "network inspect") printf "%s\\n" "$SUBNETS" ;;\n'
                '  "ps --format") printf "%s\\n" "${PORTS:-}" ;;\n'
                '  "info --format") echo 2 ;;\n'
                "esac\n"
            )
            (bin_dir / "k3d").write_text(
                "#!/bin/sh\n"
                'printf "%s\\n" "$*" >>"$CALLS"\n'
                '[ "$1 $2" = "cluster create" ] || exit 0\n'
                'line=$(head -n 1 "$FAILURES")\n'
                '[ -n "$line" ] || exit 0\n'
                'sed -i 1d "$FAILURES"\n'
                'printf "%s\\n" "$line" >&2\n'
                "exit 1\n"
            )
            for tool in ("docker", "k3d"):
                (bin_dir / tool).chmod(0o755)
            script = (
                f'source "{E2E / "run.sh"}"\n'
                "platform_require_kernel_defaults() { :; }\n"
                f'state_dir="{tmp}"\n'
                f"{call}"
            )
            env = {k: v for k, v in os.environ.items() if k != "BAUKIT_E2E_SUBNET"}
            env.update(
                PATH=f"{bin_dir}:{os.environ['PATH']}",
                SUBNETS=subnets,
                CALLS=str(calls),
                FAILURES=str(failures),
                **(extra_env or {}),
            )
            result = process_runner.run_process(
                ["bash", "-c", script], env=env, capture_output=True, text=True, check=False
            )
            creates = [
                re.search(r"--subnet (\S+)", line).group(1)
                for line in (calls.read_text() if calls.exists() else "").splitlines()
                if line.startswith("cluster create")
            ]
            return result, creates

    def test_an_overlapping_wider_pool_is_skipped(self):
        result, _ = self.run_runner_function("free_subnet", "10.89.96.0/20 fd00:89::/64 ")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "10.89.112.0/24\n")

    def test_an_exact_match_is_skipped(self):
        result, _ = self.run_runner_function("free_subnet", "10.89.100.0/24 172.17.0.0/16 ")
        self.assertEqual(result.stdout, "10.89.101.0/24\n")

    def test_a_gateway_bound_by_a_leaked_container_is_skipped(self):
        result, _ = self.run_runner_function(
            "free_subnet",
            "",
            extra_env={
                "PORTS": "10.89.100.1:9418->9418/tcp, 127.0.0.1:8080->80/tcp\n"
                "10.89.101.7:80->80/tcp"
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "10.89.101.0/24\n")

    def test_an_overlapping_slash_16_leaves_no_candidate(self):
        result, _ = self.run_runner_function("free_subnet", "10.89.0.0/16 ")
        self.assertEqual(result.returncode, 1)
        self.assertIn("no free /24", result.stderr)

    def test_ipv6_pools_are_ignored(self):
        result, _ = self.run_runner_function("free_subnet", "fd00::/8 ")
        self.assertEqual(result.stdout, "10.89.100.0/24\n")

    def test_tried_subnets_are_skipped(self):
        result, _ = self.run_runner_function(
            "tried_subnets=(10.89.100.0/24)\nfree_subnet", "10.89.101.0/24 "
        )
        self.assertEqual(result.stdout, "10.89.102.0/24\n")

    def test_pool_overlap_on_create_retries_with_the_next_subnet(self):
        overlap = "Error response from daemon: Pool overlaps with other one on this address space\n"
        result, creates = self.run_runner_function(
            "create_k3d_cluster img 1 2 3", "", k3d_failures=overlap
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(creates, ["10.89.100.0/24", "10.89.101.0/24"])
        self.assertIn("retrying with the next one", result.stderr)

    def test_retries_stop_after_five_attempts(self):
        overlap = "Pool overlaps with other one on this address space\n" * 6
        result, creates = self.run_runner_function(
            "create_k3d_cluster img 1 2 3", "", k3d_failures=overlap
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(creates), 5)

    def test_other_create_errors_do_not_retry(self):
        result, creates = self.run_runner_function(
            "create_k3d_cluster img 1 2 3", "", k3d_failures="port is already allocated\n"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(creates, ["10.89.100.0/24"])
        self.assertIn("port is already allocated", result.stderr)

    def test_an_explicit_subnet_never_retries(self):
        result, creates = self.run_runner_function(
            "create_k3d_cluster img 1 2 3",
            "",
            k3d_failures="Pool overlaps with other one on this address space\n",
            extra_env={"BAUKIT_E2E_SUBNET": "10.89.150.0/24"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(creates, ["10.89.150.0/24"])


class KubeBenchScoreTests(unittest.TestCase):
    def score(self, report, section=""):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "report.txt"
            path.write_text(report)
            script = f'source "{E2E / "k3s-checks.sh"}"\nkube_bench_score "{path}" {section}'
            return process_runner.run_process(
                ["bash", "-c", script], capture_output=True, text=True, check=False
            )

    def test_reads_the_total_summary(self):
        report = (
            "== Summary master ==\n9 checks PASS\n1 checks FAIL\n0 checks WARN\n0 checks INFO\n"
            "== Summary total ==\n70 checks PASS\n12 checks FAIL\n40 checks WARN\n0 checks INFO\n"
        )
        result = self.score(report)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "70 12 40 0\n")

    def test_reads_a_named_section(self):
        report = (
            "== Summary etcd ==\n0 checks PASS\n0 checks FAIL\n7 checks WARN\n0 checks INFO\n"
            "== Summary total ==\n70 checks PASS\n12 checks FAIL\n40 checks WARN\n0 checks INFO\n"
        )
        result = self.score(report, "etcd")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "0 0 7 0\n")

    def test_the_job_account_can_only_list_service_accounts(self):
        script = f'source "{E2E / "k3s-checks.sh"}"\nkube_bench_rbac'
        result = process_runner.run_process(
            ["bash", "-c", script], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        rules = result.stdout.split("rules:\n", 1)[1].split("---", 1)[0]
        self.assertEqual(
            rules,
            '  - apiGroups: [""]\n    resources: [serviceaccounts]\n    verbs: [list]\n',
        )

    def test_a_report_without_a_total_fails(self):
        result = self.score("== Summary master ==\n9 checks PASS\n")
        self.assertEqual(result.returncode, 1)

    def test_the_job_pins_the_image_by_digest_and_runs_in_kube_system(self):
        script = f'source "{E2E / "k3s-checks.sh"}"\nkube_bench_job'
        result = process_runner.run_process(
            ["bash", "-c", script], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("namespace: kube-system", result.stdout)
        self.assertRegex(
            result.stdout, r"image: docker\.io/aquasec/kube-bench:v[\d.]+@sha256:[0-9a-f]{64}"
        )
        self.assertIn("--benchmark, k3s-cis-", result.stdout)
        self.assertIn("--skip, '1.1.20,5.1.1,5.1.3,5.1.6'", result.stdout)
        self.assertIn("serviceAccountName: kube-bench", result.stdout)
        self.assertIn("value: /opt/baukit-journal:", result.stdout)
        self.assertIn("configMap: {name: kube-bench-journal, defaultMode: 0555}", result.stdout)


COMPONENT_LOG = "".join(
    f'time=x level=info msg="Running {name} --flag=1"\n'
    for name in (
        "kube-apiserver",
        "kube-controller-manager",
        "kube-scheduler",
        "kubelet",
        "kube-proxy",
    )
)


class KubeBenchReportTests(unittest.TestCase):
    def run_report(self, docker_log, report):
        with tempfile.TemporaryDirectory() as tmp:
            stub = Path(tmp)
            (stub / "log").write_text(docker_log)
            (stub / "report").write_text(report)
            (stub / "docker").write_text(f'#!/bin/sh\ncat "{stub}/log"\n')
            (stub / "kubectl").write_text(
                f'#!/bin/sh\ncase "$*" in *logs*) cat "{stub}/report";;'
                " *apply*) cat >/dev/null;; esac\n"
            )
            for name in ("docker", "kubectl"):
                (stub / name).chmod(0o755)
            script = f'source "{E2E / "k3s-checks.sh"}"\nkube_bench_report "{tmp}" server-0'
            env = {**os.environ, "PATH": f"{tmp}:{os.environ['PATH']}"}
            return process_runner.run_process(
                ["bash", "-c", script], capture_output=True, text=True, check=False, env=env
            )

    @staticmethod
    def summary(fail):
        return (
            "[FAIL] 1.2.1 Ensure anonymous auth is off\n" * min(fail, 1)
            + "== Summary etcd ==\n7 checks PASS\n0 checks FAIL\n0 checks WARN\n0 checks INFO\n"
            + f"== Summary total ==\n67 checks PASS\n{fail} checks FAIL\n"
            + "44 checks WARN\n19 checks INFO\n"
        )

    def test_passes_without_fail(self):
        result = self.run_report(COMPONENT_LOG, self.summary(0))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("total: 67 PASS, 0 FAIL", result.stdout)

    def test_fails_on_a_fail_and_lists_it(self):
        result = self.run_report(COMPONENT_LOG, self.summary(1))
        self.assertEqual(result.returncode, 1)
        self.assertIn("[FAIL] 1.2.1", result.stderr)

    def test_fails_when_a_component_logged_no_args(self):
        log = COMPONENT_LOG.replace("Running kubelet", "Running nothing")
        result = self.run_report(log, self.summary(0))
        self.assertEqual(result.returncode, 1)
        self.assertIn('no "Running kubelet" line', result.stderr)


class PostgresReadinessTests(unittest.TestCase):
    def test_postgres_kustomizations_wait_for_a_ready_cnpg_cluster(self):
        local = E2E.parent / "overlays/local/reconciliation.yaml"
        for path in (RECONCILIATION, local):
            documents = path.read_text().split("\n---\n")
            postgres = [d for d in documents if "components/postgres\n" in d]
            self.assertEqual(len(postgres), 1, path)
            self.assertRegex(
                postgres[0],
                r"kind: Cluster\n\s+current: has\(status\.conditions\) && "
                r"status\.conditions\.exists\(e, e\.type == 'Ready' && e\.status == 'True'\)",
            )


SINCE = "2026-11-15T16:00:00Z"
SHELL_RULE = "Baukit shell or package manager in app container"
SHADOW_RULE = "Read sensitive file untrusted"
EXEC_RULE = "Baukit exec into protected platform namespace"


def falco_event(priority, rule, namespace, process, pod=None):
    return {
        "time": "2026-11-15T16:00:00Z",
        "priority": priority,
        "rule": rule,
        "output": f"{priority} {rule}",
        "output_fields": {"k8s.ns.name": namespace, "proc.name": process, "k8s.pod.name": pod},
    }


class FalcoEventsTests(unittest.TestCase):
    EVENTS = [
        falco_event(
            "Notice", "Redirect STDOUT/STDIN to Network Connection in Container", None, "sh"
        ),
        falco_event("Warning", SHADOW_RULE, "chainsaw-a", "runc:[1:CHILD]", "shadow-reader"),
        falco_event("Warning", SHADOW_RULE, "chainsaw-a", "cat", "shadow-reader"),
        falco_event("Critical", SHELL_RULE, "e2e-tenant", "sh", "echo-api-7d9-x2"),
    ]

    def run_events(self, *args):
        lines = [json.dumps(event) for event in self.EVENTS]
        loki = {"data": {"result": [{"values": [["1", line] for line in lines]}]}}
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "falco.log").write_text("Sun Nov 15 falco banner\n" + "\n".join(lines) + "\n")
            Path(tmp, "loki.json").write_text(json.dumps(loki))
            kubectl = Path(tmp) / "kubectl"
            kubectl.write_text(
                "#!/bin/sh\n"
                'echo "$*" >>"$STUB/calls"\n'
                'case "$*" in\n'
                '  *"get pods"*) echo pod/falco-a ;;\n'
                '  *logs*) cat "$STUB/falco.log" ;;\n'
                '  *"get --raw"*) cat "$STUB/loki.json" ;;\n'
                "  *) exit 9 ;;\n"
                "esac\n"
            )
            kubectl.chmod(0o755)
            env = {**os.environ, "PATH": f"{tmp}:{os.environ['PATH']}", "STUB": tmp}
            result = process_runner.run_process(
                [str(E2E / "tests/falco/falco-events.sh"), *args],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            calls = Path(tmp, "calls")
            self.calls = calls.read_text() if calls.exists() else ""
        return result

    def rules(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return [json.loads(line)["rule"] for line in result.stdout.splitlines()]

    def query(self, source, priority, *filters):
        return self.run_events(
            "--source", source, "--since", SINCE, "--min-priority", priority, *filters
        )

    def test_soak_floor_drops_notice_events(self):
        rules = self.rules(self.query("falco", "Warning"))
        self.assertEqual(len(rules), 3)
        self.assertNotIn("Redirect STDOUT/STDIN to Network Connection in Container", rules)

    def test_loki_and_falco_sources_agree(self):
        falco = self.query("falco", "Debug").stdout
        loki = self.query("loki", "Debug").stdout
        self.assertEqual(falco, loki)
        self.assertEqual(len(loki.splitlines()), 4)

    def test_loki_window_starts_at_since(self):
        self.query("loki", "Debug")
        since_s = int(datetime.fromisoformat(SINCE.replace("Z", "+00:00")).timestamp())
        self.assertIn(f"start={since_s * 1_000_000_000}&", self.calls)

    def test_loki_triggers_start_at_falco_pod_start(self):
        chainsaw = (E2E / "tests/falco/chainsaw-test.yaml").read_text()
        falco_start = (
            "kubectl -n falco get pods -l app.kubernetes.io/name=falco,pod-template-generation"
        )
        self.assertNotIn("since=$(date", chainsaw)
        self.assertEqual(chainsaw.count(f"since=$({falco_start}"), 3)

    def test_process_filter_skips_the_runc_mount_read(self):
        filters = ("--rule", SHADOW_RULE, "--namespace", "chainsaw-a", "--process", "cat")
        self.assertEqual(len(self.rules(self.query("loki", "Warning", *filters))), 1)

    def test_namespace_filter(self):
        filters = ("--rule", SHELL_RULE, "--namespace", "other")
        self.assertEqual(self.rules(self.query("loki", "Critical", *filters)), [])

    def test_excluded_trigger_pods_leave_other_events(self):
        filters = ("--exclude-pod", "shadow-reader", "--exclude-pod", "echo-api-7d9-x2")
        self.assertEqual(self.rules(self.query("falco", "Warning", *filters)), [])
        only_echo = self.query("falco", "Warning", "--exclude-pod", "echo-api-7d9-x2")
        self.assertEqual(self.rules(only_echo), [SHADOW_RULE, SHADOW_RULE])

    def test_noise_check_excludes_the_concurrent_suites_exec_pods(self):
        chainsaw = (E2E / "tests/falco/chainsaw-test.yaml").read_text()
        step = chainsaw.split("name: no-warning-events-outside-the-triggers", 1)[1]
        excluded = re.findall(r"--exclude-pod ([^\s)]+)", step.split("if [", 1)[0])
        probe = (E2E / "tests/netpol/probe.py").read_text()
        prefix = re.search(r'^POD_PREFIX = "([^"]+)"$', probe, re.MULTILINE)
        self.assertIsNotNone(prefix)
        self.assertIn(f"{prefix.group(1)}-", excluded)
        self.assertIn("e2e-backup-", excluded)

    def test_pod_filter_matches_the_exact_pod(self):
        filters = ("--rule", SHADOW_RULE, "--pod", "shadow-reader")
        self.assertEqual(len(self.rules(self.query("loki", "Warning", *filters))), 2)
        self.assertIn("%7C%3D%20%22shadow-reader%22", self.calls)
        self.assertEqual(self.rules(self.query("falco", "Warning", "--pod", "shadow")), [])

    def test_trigger_checks_name_this_runs_pod(self):
        chainsaw = (E2E / "tests/falco/chainsaw-test.yaml").read_text()
        self.assertIn('--namespace e2e-tenant --pod "$pod")', chainsaw)
        self.assertIn("--pod shadow-reader --process cat", chainsaw)

    def test_kubernetes_only_drops_events_without_a_namespace(self):
        host = falco_event("Critical", "Drop and execute new binary in container", None, "check")
        self.EVENTS = [*FalcoEventsTests.EVENTS, host]
        everything = self.rules(self.query("falco", "Warning"))
        self.assertIn("Drop and execute new binary in container", everything)
        cluster_only = self.rules(self.query("falco", "Warning", "--kubernetes-only", "true"))
        self.assertEqual(cluster_only, [SHADOW_RULE, SHADOW_RULE, SHELL_RULE])

    def test_node_container_ids_keep_this_clusters_node_events(self):
        foreign_rule, node_rule = "Drop and execute new binary in container", "Fileless execution"
        foreign = falco_event("Critical", foreign_rule, None, "check")
        foreign["output_fields"]["container.id"] = "e0061c7e0721"
        node = falco_event("Critical", node_rule, None, "k3s")
        node["output_fields"]["container.id"] = "5bde0017ed33"
        self.EVENTS = [*FalcoEventsTests.EVENTS, foreign, node]
        scoped = self.query(
            "falco",
            "Warning",
            "--kubernetes-only",
            "true",
            "--node-container-id",
            "5bde0017ed33",
            "--exclude-pod",
            "shadow-reader",
        )
        self.assertEqual(self.rules(scoped), [SHELL_RULE, node_rule])

    def test_noise_check_scopes_events_to_this_cluster(self):
        chainsaw = (E2E / "tests/falco/chainsaw-test.yaml").read_text()
        step = chainsaw.split("name: no-warning-events-outside-the-triggers", 1)[1]
        step = step.split("- name:", 1)[0]
        self.assertIn('scope="--kubernetes-only true"', step)
        self.assertIn("--exclude-pod netpol-probe- $scope)", step)
        cluster = "${BAUKIT_E2E_CLUSTER:-baukit-platform-e2e}"
        self.assertIn(f'docker ps -q --filter "label=k3d.cluster={cluster}"', step)
        self.assertIn('scope="$scope --node-container-id $node"', step)
        self.assertIn("case ${BAUKIT_E2E_PROFILE:-} in\n                persistent)", step)
        self.assertNotIn("BAUKIT_E2E_PROFILE", (E2E / "run.sh").read_text())

    def test_excluded_prefix_drops_netpol_probe_execs(self):
        probe_exec = falco_event("Critical", EXEC_RULE, "postgres", "sh", "netpol-probe-19")
        self.EVENTS = [*FalcoEventsTests.EVENTS, probe_exec]
        filtered = self.query("falco", "Warning", "--exclude-pod", "netpol-probe-")
        self.assertNotIn(EXEC_RULE, self.rules(filtered))
        self.assertIn(EXEC_RULE, self.rules(self.query("falco", "Warning")))

    def test_unknown_source_exits_2(self):
        self.assertEqual(self.query("journal", "Warning").returncode, 2)

    def test_missing_since_exits_2(self):
        self.assertEqual(self.run_events("--source", "falco").returncode, 2)


class RegistryMirrorTests(unittest.TestCase):
    def source(self, call):
        script = f'source "{E2E.parent / "registry-mirrors.sh"}"\n{call}'
        result = process_runner.run_process(["bash", "-c", script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_every_upstream_routes_to_its_mirror(self):
        config = self.source("registry_mirrors_k3s_config")
        for host in ("docker.io", "ghcr.io"):
            mirror = f"http://k3d-baukit-mirror-{host.replace('.', '-')}:5000"
            self.assertIn(f"  {host}:\n    endpoint:\n      - {mirror}\n", config)

    def test_kyverno_images_go_through_the_ghcr_mirror(self):
        k3s = self.source("registry_mirrors_k3s_config")
        trivy = self.source("registry_mirrors_trivy_config")
        ghcr_mirror = "http://k3d-baukit-mirror-ghcr-io:5000"
        self.assertIn(f"  reg.kyverno.io:\n    endpoint:\n      - {ghcr_mirror}\n", k3s)
        self.assertIn("    reg.kyverno.io:\n      - localhost:5702\n", trivy)
        cluster_args = self.source("registry_mirrors_cluster_args")
        self.assertNotIn("baukit-mirror-reg-kyverno-io", cluster_args)

    def test_the_cluster_uses_every_mirror_it_routes_to(self):
        config = self.source("registry_mirrors_k3s_config")
        args = self.source("registry_mirrors_cluster_args").splitlines()
        pairs = zip(args[::2], args[1::2], strict=True)
        used = {f"http://{arg}" for flag, arg in pairs if flag == "--registry-use"}
        lines = [line.strip() for line in config.splitlines()]
        endpoints = {line[2:] for line in lines if line.startswith("- ")}
        self.assertEqual(used, endpoints)

    def test_detach_disconnects_every_mirror_from_the_cluster_network(self):
        stub = 'docker() { printf "%s\\n" "$*" >&3; }'
        out = self.source(f"{stub}\nregistry_mirrors_detach k3d-x 3>&1")
        self.assertIn("network disconnect k3d-x k3d-baukit-mirror-docker-io", out)
        self.assertEqual(out.count("network disconnect k3d-x "), 4)

    def test_prune_removes_only_layer_links_without_blob_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "docker/registry/v2"
            kept, gone = "a1" + "0" * 62, "b2" + "0" * 62
            for repo, hexdigest in (("library/redis", kept), ("kyverno/kyverno-cli", gone)):
                layer = root / "repositories" / repo / "_layers/sha256" / hexdigest
                layer.mkdir(parents=True)
                (layer / "link").write_text(f"sha256:{hexdigest}")
            data = root / "blobs/sha256" / kept[:2] / kept / "data"
            data.parent.mkdir(parents=True)
            data.write_text("blob")
            script = "${registry_mirror_prune_script/\\/var\\/lib\\/registry/" + tmp + "}"
            out = self.source(f'sh -c "{script}"')
            self.assertEqual(out, f"repositories/kyverno/kyverno-cli sha256:{gone}\n")
            layers = "_layers/sha256"
            self.assertTrue((root / "repositories/library/redis" / layers / kept).exists())
            self.assertFalse((root / "repositories/kyverno/kyverno-cli" / layers / gone).exists())

    def test_prune_restarts_the_mirror_only_when_it_removed_links(self):
        for found, restarted in (("repositories/x sha256:ab", True), ("", False)):
            stub = (
                'docker() { printf "%s\\n" "$1" >&3; '
                f'[[ $1 == exec ]] && printf "%s" "{found}"; return 0; }}'
            )
            out = self.source(f"{stub}\nregistry_mirror_prune k3d-m 3>&1 2>/dev/null")
            self.assertEqual("restart" in out.split(), restarted, out)

    def test_the_mirror_image_is_pinned_by_digest(self):
        image = self.source('printf %s "$registry_mirror_image"')
        self.assertRegex(image, r"^docker\.io/library/registry:[0-9.]+@sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
