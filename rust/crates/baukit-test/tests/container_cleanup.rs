use std::{collections::BTreeMap, error::Error, process::Command, time::SystemTime};

#[tokio::test]
#[ignore = "requires Docker"]
async fn postgres_has_cleanup_labels() -> Result<(), Box<dyn Error>> {
    let before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();
    let postgres = baukit_test::start_postgres().await?;
    let output = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{json .Config.Labels}}",
            postgres.container().id(),
        ])
        .output()?;
    assert!(output.status.success());
    let labels: BTreeMap<String, String> = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        labels.get("dev.baukit.test").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        labels["dev.baukit.test.pid"],
        std::process::id().to_string()
    );
    assert_eq!(
        labels["dev.baukit.test.binary"],
        std::env::current_exe()?
            .file_name()
            .ok_or("binary name")?
            .to_string_lossy()
    );
    assert_eq!(
        labels.get("dev.baukit.test.manifest-dir"),
        std::env::var("CARGO_MANIFEST_DIR").ok().as_ref()
    );
    #[cfg(target_os = "linux")]
    {
        let namespace = std::fs::read_link("/proc/self/ns/pid")?;
        assert_eq!(
            labels["dev.baukit.test.pid-ns"],
            namespace.to_str().ok_or("namespace text")?
        );
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))?;
        assert_eq!(
            labels["dev.baukit.test.host"],
            hostname.trim_end_matches('\n')
        );
    }
    let started: u64 = labels["dev.baukit.test.started-at"].parse()?;
    let after = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();
    assert!((before..=after).contains(&started));
    Ok(())
}

#[cfg(unix)]
mod watchdog {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::process::ExitStatusExt,
        process::{Child, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
    const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    const SIGTERM: i32 = 15;

    struct ChildContainer {
        child: Child,
        id: Option<String>,
    }

    impl Drop for ChildContainer {
        fn drop(&mut self) {
            if self
                .child
                .try_wait()
                .expect("check watchdog child")
                .is_none()
            {
                self.child.kill().expect("kill watchdog child");
            }
            self.child.wait().expect("reap watchdog child");
            if let Some(id) = &self.id {
                let output = Command::new("docker")
                    .args(["rm", "--force", "--volumes", id])
                    .output()
                    .expect("clean up child container");
                if !output.status.success()
                    && !String::from_utf8_lossy(&output.stderr).contains("No such container")
                {
                    eprintln!(
                        "child container cleanup failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }

    fn child() -> Result<(), Box<dyn Error>> {
        tokio::runtime::Runtime::new()?.block_on(async {
            let postgres = baukit_test::start_postgres().await?;
            println!("container-id={}", postgres.container().id());
            std::io::stdout().flush()?;
            std::future::pending::<()>().await;
            drop(postgres);
            Ok::<(), Box<dyn Error>>(())
        })
    }

    #[test]
    #[ignore = "requires Docker and Unix signals"]
    fn sigterm_removes_container_and_terminates_child() -> Result<(), Box<dyn Error>> {
        if std::env::var_os("BAUKIT_WATCHDOG_CHILD").is_some() {
            return child();
        }
        let child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "watchdog::sigterm_removes_container_and_terminates_child",
                "--ignored",
                "--nocapture",
            ])
            .env("BAUKIT_WATCHDOG_CHILD", "1")
            .env_remove("TESTCONTAINERS_COMMAND")
            .stdout(Stdio::piped())
            .spawn()?;
        let mut guard = ChildContainer { child, id: None };
        let stdout = guard.child.stdout.take().ok_or("child stdout")?;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(id) = line.strip_prefix("container-id=") {
                    sender.send(id.to_owned()).expect("send container id");
                    break;
                }
            }
        });
        let id = receiver.recv_timeout(STARTUP_TIMEOUT)?;
        assert!(id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        guard.id = Some(id.clone());
        let status = Command::new("kill")
            .args(["-TERM", &guard.child.id().to_string()])
            .status()?;
        assert!(status.success());
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        loop {
            if let Some(status) = guard.child.try_wait()? {
                assert_eq!(status.signal(), Some(SIGTERM));
                let output = Command::new("docker")
                    .args([
                        "ps",
                        "--all",
                        "--quiet",
                        "--no-trunc",
                        "--filter",
                        &format!("id={id}"),
                    ])
                    .output()?;
                assert!(output.status.success(), "Docker container lookup failed");
                assert!(
                    output.stdout.is_empty(),
                    "SIGTERM left the container behind"
                );
                guard.id = None;
                return Ok(());
            }
            assert!(
                Instant::now() < deadline,
                "watchdog did not terminate the child within {CLEANUP_TIMEOUT:?}"
            );
            thread::sleep(POLL_INTERVAL);
        }
    }
}
