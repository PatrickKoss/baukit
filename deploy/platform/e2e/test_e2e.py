import importlib.util
import json
import os
import re
import sys
import tempfile
import time
import unittest
from pathlib import Path

E2E = Path(__file__).resolve().parent
PLATFORM = E2E.parent
REPO = PLATFORM.parent.parent
RECONCILIATION = E2E / "overlay/reconciliation.yaml"
NOT_BASES = {"components", "e2e", "lib", "lint", "overlays", "schemas"}
LEFT_OUT = {"hcloud", "posthog", "redis"}
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
        for component in sorted(path.parent for path in local.glob("*/kustomization.yaml")):
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
                f'source <(sed -n "/^stalled_releases() {{/,/^}}/p" "{E2E / "run.sh"}")\n'
                "stalled_releases"
            )
            result = process_runner.run_process(
                ["bash", "-c", script], capture_output=True, text=True
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "falco/falco\n")


class FailedKustomizationTests(unittest.TestCase):
    def run_function(self, function, setup, call):
        script = f'{setup}\nsource <(sed -n "/^{function}() {{/,/^}}/p" "{E2E / "run.sh"}")\n{call}'
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


class RegistryMirrorTests(unittest.TestCase):
    def source(self, call):
        script = f'source "{E2E.parent / "registry-mirrors.sh"}"\n{call}'
        result = process_runner.run_process(["bash", "-c", script], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_every_upstream_routes_to_its_mirror(self):
        config = self.source("registry_mirrors_k3s_config")
        for host in ("docker.io", "reg.kyverno.io"):
            mirror = f"http://k3d-baukit-mirror-{host.replace('.', '-')}:5000"
            self.assertIn(f"  {host}:\n    endpoint:\n      - {mirror}\n", config)

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
        self.assertEqual(out.count("network disconnect k3d-x "), 5)

    def test_the_mirror_image_is_pinned_by_digest(self):
        image = self.source('printf %s "$registry_mirror_image"')
        self.assertRegex(image, r"^docker\.io/library/registry:[0-9.]+@sha256:[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
