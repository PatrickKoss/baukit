use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use crate::{
    Capabilities, Component, Components,
    doctor_auth::validate_mobile_qa_build_flag,
    doctor_frontend::{
        pwa_app_directory, validate_mobile_router_configuration, validate_pwa_worker_build,
    },
    doctor_host::{
        DoctorCommandOutput, DoctorHost, diagnose_docker, diagnose_ssh_agent, probe_git_dependency,
    },
    doctor_ports::{has_mismatched_loopback_port, resolve_makefile_variables},
};

struct ExpectedCommand {
    program: &'static str,
    args: Vec<&'static str>,
    result: io::Result<DoctorCommandOutput>,
}

struct FakeHost {
    ssh_auth_sock: Option<OsString>,
    socket: bool,
    commands: RefCell<VecDeque<ExpectedCommand>>,
}

impl FakeHost {
    fn new(ssh_auth_sock: Option<&str>, socket: bool, commands: Vec<ExpectedCommand>) -> Self {
        Self {
            ssh_auth_sock: ssh_auth_sock.map(OsString::from),
            socket,
            commands: RefCell::new(commands.into()),
        }
    }

    fn assert_finished(&self) {
        assert!(self.commands.borrow().is_empty());
    }
}

impl DoctorHost for FakeHost {
    fn env_var_os(&self, name: &str) -> Option<OsString> {
        assert_eq!(name, "SSH_AUTH_SOCK");
        self.ssh_auth_sock.clone()
    }

    fn is_socket(&self, path: &Path) -> bool {
        assert_eq!(path, PathBuf::from("/agent.sock"));
        self.socket
    }

    fn run_command(
        &self,
        program: &str,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> io::Result<DoctorCommandOutput> {
        assert!(current_dir.is_none());
        let expected = self
            .commands
            .borrow_mut()
            .pop_front()
            .expect("unexpected command");
        assert_eq!(program, expected.program);
        assert_eq!(
            args,
            &expected
                .args
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
        expected.result
    }
}

fn output(code: i32) -> io::Result<DoctorCommandOutput> {
    Ok(DoctorCommandOutput {
        success: code == 0,
        code: Some(code),
        stderr: String::new(),
    })
}

#[test]
fn mobile_env_files_must_not_set_the_qa_build_flag() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("mobile"))?;
    fs::write(
        root.path().join("mobile/.env.example"),
        "EXPO_PUBLIC_API_URL=http://localhost:8080\nBAUKIT_QA_BUILD_NOTE=1\n",
    )?;
    let mut failures = Vec::new();
    validate_mobile_qa_build_flag(root.path(), &mut failures)?;
    assert!(failures.is_empty(), "{failures:?}");

    fs::write(
        root.path().join("mobile/.env"),
        "export BAUKIT_QA_BUILD=1\n",
    )?;
    fs::write(
        root.path().join("mobile/.env.example"),
        "BAUKIT_QA_BUILD=\n",
    )?;
    validate_mobile_qa_build_flag(root.path(), &mut failures)?;
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures[0].starts_with("mobile/.env.example sets `BAUKIT_QA_BUILD`"));
    assert!(failures[1].starts_with("mobile/.env sets `BAUKIT_QA_BUILD`"));
    Ok(())
}

#[test]
fn mobile_router_configuration_requires_the_entry_point_and_scheme() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("mobile"))?;
    fs::write(
        root.path().join("mobile/package.json"),
        "{\"main\":\"expo/AppEntry.js\"}\n",
    )?;
    fs::write(
        root.path().join("mobile/app.config.ts"),
        "export default { slug: 'product' };\n",
    )?;
    let mut successes = Vec::new();
    let mut failures = Vec::new();

    validate_mobile_router_configuration(root.path(), &mut successes, &mut failures)?;

    assert!(successes.is_empty());
    assert!(
        failures
            .iter()
            .any(|failure| failure.contains("expo-router/entry"))
    );
    assert!(
        failures
            .iter()
            .any(|failure| failure.contains("deep-link scheme"))
    );
    Ok(())
}

#[test]
fn pwa_app_directory_prefers_the_web_app_and_falls_back_to_the_expo_web_export() {
    let capabilities = |web, mobile| {
        let mut components = Components::from([Component::Backend, Component::Pwa]);
        components.set(Component::Web, web);
        components.set(Component::Mobile, mobile);
        Capabilities {
            analytics: crate::AnalyticsAdapter::Posthog,
            components,
            auth: None,
        }
    };

    assert_eq!(pwa_app_directory(&capabilities(true, true)), Some("web"));
    assert_eq!(pwa_app_directory(&capabilities(true, false)), Some("web"));
    assert_eq!(
        pwa_app_directory(&capabilities(false, true)),
        Some("mobile")
    );
    assert_eq!(pwa_app_directory(&capabilities(false, false)), None);
}

#[test]
fn pwa_worker_build_is_checked_in_the_expo_app_without_a_web_app() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("mobile/scripts"))?;
    fs::write(
        root.path().join("mobile/package.json"),
        r#"{ "scripts": { "build:sw": "node scripts/build-sw.mjs" } }"#,
    )?;
    let mut successes = Vec::new();
    let mut failures = Vec::new();

    validate_pwa_worker_build(root.path(), "mobile", &mut successes, &mut failures)?;

    assert!(successes.is_empty());
    assert_eq!(
        failures,
        [
            "the PWA capability requires the mobile `build:sw:check` script",
            "the PWA capability requires the mobile `@baukit/pwa-web` dependency",
            "the PWA capability requires `mobile/scripts/build-sw.mjs`",
        ]
    );
    Ok(())
}

#[test]
fn pwa_worker_build_requires_supported_artifact_wiring() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("web/scripts"))?;
    fs::write(
        root.path().join("web/package.json"),
        r#"{
            "scripts": {
                "build:sw": "node scripts/build-sw.mjs",
                "build:sw:check": "node scripts/build-sw.mjs --check"
            },
            "dependencies": { "@baukit/pwa-web": "0.3.0" }
        }"#,
    )?;
    fs::write(
        root.path().join("web/scripts/build-sw.mjs"),
        "import.meta.resolve('@baukit/pwa-web/worker');\n",
    )?;
    let mut successes = Vec::new();
    let mut failures = Vec::new();

    validate_pwa_worker_build(root.path(), "web", &mut successes, &mut failures)?;

    assert!(failures.is_empty());
    assert_eq!(
        successes,
        ["web PWA worker build uses the supported Baukit artifact"]
    );

    fs::write(
        root.path().join("web/scripts/build-sw.mjs"),
        "import.meta.resolve('@baukit/pwa-web');\n",
    )?;
    validate_pwa_worker_build(root.path(), "web", &mut Vec::new(), &mut failures)?;
    assert!(
        failures
            .iter()
            .any(|failure| failure.contains("supported `@baukit/pwa-web/worker` artifact"))
    );
    Ok(())
}

#[test]
fn makefile_defaults_and_immediate_assignments_follow_definition_order() {
    let source = r#"PORT ?= 8280 # API default
PORT ?= 9999
EARLY := $(PORT)
LATE = ${PORT}
PORT = 8281
ALIAS = ${FORWARD}
FORWARD = 9290
	run api=$(EARLY) ops=${ALIAS} late=$(LATE)"#;
    assert_eq!(
        resolve_makefile_variables(source).lines().last(),
        Some("\trun api=8280 ops=9290 late=8281")
    );
}

#[test]
fn makefile_resolution_preserves_shell_expressions_and_unknown_variables() {
    let source = r#"export PORT = 8280
	run $$(PORT) $${PORT} $(UNKNOWN) ${UNKNOWN} $plain $(shell printf 9999) $(BROKEN"#;
    assert_eq!(
        resolve_makefile_variables(source).lines().last(),
        source.lines().last()
    );
}

#[test]
fn makefile_resolution_ignores_recipe_assignments_and_leaves_cycles_unresolved() {
    let source = r#"FIRST = $(SECOND)
SECOND = ${FIRST}
PORT = 8280
run:
	PORT = 9999
	run api=$(PORT) cycle=$(FIRST)"#;
    let resolved = resolve_makefile_variables(source);
    let command = resolved.lines().last().expect("recipe command");
    assert!(command.starts_with("\trun api=8280 cycle=$"));
}

#[test]
fn localhost_check_allows_an_expected_service_port_with_a_frontend_origin() {
    let source = r#"
        const issuer = "http://localhost:8081/realms/product";
        const origin = "http://localhost:5173";
    "#;
    assert!(!has_mismatched_loopback_port(source, 8081, &[], true));
    assert!(has_mismatched_loopback_port(source, 8181, &[], true));
}

#[test]
fn ssh_agent_reports_missing_and_non_socket_values_without_running_commands() {
    for (socket, is_socket, expected) in [
        (None, false, "SSH_AUTH_SOCK is unset"),
        (
            Some("/agent.sock"),
            false,
            "SSH_AUTH_SOCK does not point to a socket",
        ),
    ] {
        let host = FakeHost::new(socket, is_socket, Vec::new());
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        assert!(!diagnose_ssh_agent(&host, &mut successes, &mut failures));
        assert!(successes.is_empty());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains(expected));
        assert!(failures[0].contains("ssh-add"));
        assert!(!failures[0].contains("/agent.sock"));
        host.assert_finished();
    }
}

#[test]
fn ssh_agent_distinguishes_no_identities_from_an_unusable_agent() {
    for (code, expected) in [(1, "no loaded identities"), (2, "agent is unusable")] {
        let host = FakeHost::new(
            Some("/agent.sock"),
            true,
            vec![ExpectedCommand {
                program: "ssh-add",
                args: vec!["-l"],
                result: output(code),
            }],
        );
        let mut successes = Vec::new();
        let mut failures = Vec::new();
        assert!(!diagnose_ssh_agent(&host, &mut successes, &mut failures));
        assert!(failures[0].contains(expected));
        assert!(failures[0].contains("ssh-add"));
        host.assert_finished();
    }
}

#[test]
fn ssh_agent_accepts_a_socket_with_a_loaded_identity() {
    let host = FakeHost::new(
        Some("/agent.sock"),
        true,
        vec![ExpectedCommand {
            program: "ssh-add",
            args: vec!["-l"],
            result: output(0),
        }],
    );
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    assert!(diagnose_ssh_agent(&host, &mut successes, &mut failures));
    assert!(failures.is_empty());
    assert!(successes[0].contains("loaded identity"));
    host.assert_finished();
}

#[test]
fn git_probe_uses_only_the_remote_and_tag() {
    let host = FakeHost::new(
        None,
        false,
        vec![ExpectedCommand {
            program: "git",
            args: vec![
                "ls-remote",
                "--exit-code",
                "--tags",
                "ssh://git@example.test/baukit.git",
                "v1.2.3",
            ],
            result: output(0),
        }],
    );
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    probe_git_dependency(
        &host,
        "ssh://git@example.test/baukit.git",
        "v1.2.3",
        &mut successes,
        &mut failures,
    );
    assert!(failures.is_empty());
    assert!(successes[0].contains("current SSH identity"));
    host.assert_finished();
}

#[test]
fn docker_diagnostic_checks_the_daemon_then_buildkit() {
    let host = FakeHost::new(
        None,
        false,
        vec![
            ExpectedCommand {
                program: "docker",
                args: vec!["version", "--format", "{{.Server.Version}}"],
                result: output(0),
            },
            ExpectedCommand {
                program: "docker",
                args: vec!["buildx", "version"],
                result: output(0),
            },
        ],
    );
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    assert!(diagnose_docker(&host, &mut successes, &mut failures));
    assert!(failures.is_empty());
    assert!(successes[0].contains("BuildKit"));
    host.assert_finished();
}

#[test]
fn docker_diagnostic_reports_a_missing_buildkit_plugin() {
    let host = FakeHost::new(
        None,
        false,
        vec![
            ExpectedCommand {
                program: "docker",
                args: vec!["version", "--format", "{{.Server.Version}}"],
                result: output(0),
            },
            ExpectedCommand {
                program: "docker",
                args: vec!["buildx", "version"],
                result: output(1),
            },
        ],
    );
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    assert!(!diagnose_docker(&host, &mut successes, &mut failures));
    assert!(successes.is_empty());
    assert!(failures[0].contains("BuildKit is unavailable"));
    host.assert_finished();
}
