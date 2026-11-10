use std::{
    error::Error,
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use baukit_config::Environment;

use {{ context.app_crate }}_bin::{ProductConfig, config_loader};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const CHILD_MODE: &str = "BAUKIT_CONFIG_LOADER_TEST_MODE";
const ENV_PREFIX: &str = "{{ context.app_env }}";
const ORIGINS: &str = r#"["https://app.example.com", "http://127.0.0.1:5173"]"#;
{% if context.auth_enabled %}const REDIRECT_URIS: &str = r#"["https://app.example.com/", "https://app.example.com/callback"]"#;
{% endif %}{% if context.mcp %}const HOSTS: &str = r#"["mcp.example.com", "localhost:8080"]"#;
{% endif %}
#[test]
fn parses_json_array_environment_values() -> Result<(), Box<dyn Error>> {
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let config = config_loader(Environment::Staging)?
            .without_local_file()
            .without_dotenv()
            .load::<ProductConfig>()?;
        let origins = if mode == "empty" {
            Vec::new()
        } else {
            vec!["https://app.example.com", "http://127.0.0.1:5173"]
        };
        assert_eq!(config.http.cors_allowed_origins, origins);
{% if context.platform %}        let features = if mode == "empty" {
            Vec::new()
        } else {
            vec!["export", "offline"]
        };
        assert_eq!(
            config.product.platform.standalone_default.features,
            features
        );
{% endif %}{% if context.auth_enabled %}        assert_eq!(config.product.auth.authorized_parties, origins);
        if mode != "empty" {
            assert_eq!(
                config.product.auth.browser_session.redirect_uris,
                vec![
                    "https://app.example.com/",
                    "https://app.example.com/callback"
                ]
            );
        }
{% endif %}{% if context.mcp %}        assert_eq!(config.product.mcp.allowed_origins, origins);
        let hosts = if mode == "empty" {
            Vec::new()
        } else {
            vec!["mcp.example.com", "localhost:8080"]
        };
        assert_eq!(config.product.mcp.allowed_hosts, hosts);
{% endif %}        return Ok(());
    }

    for mode in ["empty", "populated"] {
        let output = configured_command(std::env::current_exe()?, mode)
            .args([
                "--exact",
                "parses_json_array_environment_values",
                "--nocapture",
            ])
            .run_bounded()?;
        assert!(
            output.status.success(),
            "configuration subprocess failed: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

{% if context.mcp %}#[test]
fn shared_mcp_environment_reaches_database_setup_in_every_command() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
{% if context.worker %}    for executable in [
        env!("CARGO_BIN_EXE_api"),
        env!("CARGO_BIN_EXE_migrate"),
        env!("CARGO_BIN_EXE_worker"),
    ] {
{% else %}    for executable in [env!("CARGO_BIN_EXE_api"), env!("CARGO_BIN_EXE_migrate")] {
{% endif %}        let output = configured_command(executable, "populated")
            .current_dir(directory.path())
            .env(format!("{ENV_PREFIX}_ENVIRONMENT"), "local")
            .env(
                format!("{ENV_PREFIX}__DATABASE__URL"),
                "postgres://127.0.0.1:invalid/config-test",
            )
            .run_bounded()?;
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Configuration(InvalidPort)"),
            "command did not reach database setup: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

{% endif %}struct CommandOutput {
    status: ExitStatus,
    stderr: Vec<u8>,
}

trait RunBounded {
    fn run_bounded(&mut self) -> Result<CommandOutput, Box<dyn Error>>;
}

impl RunBounded for Command {
    fn run_bounded(&mut self) -> Result<CommandOutput, Box<dyn Error>> {
        let mut stderr = tempfile::tempfile()?;
        let mut child = self
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr.try_clone()?)
            .spawn()?;
        let pid = child.id();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || sender.send(child.wait()));
        let status = match receiver.recv_timeout(COMMAND_TIMEOUT) {
            Ok(status) => status?,
            Err(_) => {
                let state = process_state(pid);
                Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .status()?;
                receiver.recv()??;
                return Err(format!(
                    "{self:?} ran longer than {COMMAND_TIMEOUT:?}; child state: {state}"
                )
                .into());
            }
        };
        Ok(CommandOutput {
            status,
            stderr: read_all(&mut stderr)?,
        })
    }
}

fn process_state(pid: u32) -> String {
    let read = |name: &str| std::fs::read_to_string(format!("/proc/{pid}/{name}")).ok();
    let status = read("status").unwrap_or_default();
    let state = status.lines().find(|line| line.starts_with("State:"));
    format!(
        "{} wchan={}",
        state.unwrap_or("unknown"),
        read("wchan").unwrap_or_default()
    )
}

fn read_all(file: &mut File) -> Result<Vec<u8>, Box<dyn Error>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn configured_command(executable: impl AsRef<OsStr>, mode: &str) -> Command {
    let mut command = Command::new(executable);
    let origins = if mode == "empty" { "[]" } else { ORIGINS };
    command
        .env_clear()
        .env(CHILD_MODE, mode)
        .env(format!("{ENV_PREFIX}__HTTP__CORS_ALLOWED_ORIGINS"), origins);
{% if context.auth_enabled %}    command
        .env(
            format!("{ENV_PREFIX}__AUTH__ISSUER"),
            "https://identity.example.com",
        )
        .env(format!("{ENV_PREFIX}__AUTH__CLIENT_ID"), "config-test")
        .env(format!("{ENV_PREFIX}__AUTH__AUTHORIZED_PARTIES"), origins);
    if mode != "empty" {
        command.env(
            format!("{ENV_PREFIX}__AUTH__BROWSER_SESSION__REDIRECT_URIS"),
            REDIRECT_URIS,
        );
    }
{% endif %}{% if context.mcp %}    let hosts = if mode == "empty" { "[]" } else { HOSTS };
    command
        .env(format!("{ENV_PREFIX}__MCP__ALLOWED_HOSTS"), hosts)
        .env(format!("{ENV_PREFIX}__MCP__ALLOWED_ORIGINS"), origins);
{% endif %}{% if context.platform %}    command.env(
        format!("{ENV_PREFIX}__PLATFORM__STANDALONE_DEFAULT__FEATURES"),
        if mode == "empty" {
            "[]"
        } else {
            r#"["export","offline"]"#
        },
    );
{% endif %}    command
}
