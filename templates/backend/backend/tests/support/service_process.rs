{% if not context.auth_enabled or context.worker %}use std::{
    error::Error,
    fmt,
    io::{BufRead as _, BufReader},
    net::{Ipv4Addr, SocketAddr, TcpListener},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use {{ context.app_crate }}_bin::probe_readiness;

const ENV_PREFIX: &str = "{{ context.app_env }}";
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub fn free_loopback_address() -> std::io::Result<SocketAddr> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()
}

/// Sets `{PREFIX}__{key}` the way a deployment sets a config value.
pub fn config_env(key: &str) -> String {
    format!("{ENV_PREFIX}__{key}")
}

/// A product binary started in `local` from environment variables only.
/// Dropping it kills the process.
pub struct ServiceProcess {
    child: Child,
    stderr: Arc<Mutex<String>>,
    reader: Option<JoinHandle<()>>,
}

impl ServiceProcess {
    pub fn spawn(
        binary: &str,
        workdir: &Path,
        env: &[(String, String)],
    ) -> Result<Self, Box<dyn Error>> {
        let mut child = Command::new(binary)
            .current_dir(workdir)
            .env(format!("{ENV_PREFIX}_ENVIRONMENT"), "local")
            .envs(env.iter().map(|(key, value)| (key, value)))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let pipe = child.stderr.take().ok_or("child stderr is not piped")?;
        let stderr = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&stderr);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if let Ok(mut buffer) = sink.lock() {
                    buffer.push_str(&line);
                    buffer.push('\n');
                }
            }
        });
        Ok(Self {
            child,
            stderr,
            reader: Some(reader),
        })
    }

    pub async fn wait_until_ready(&mut self, ops: SocketAddr) -> Result<(), Box<dyn Error>> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Err(self.failure(&format!("exited with {status} before ready")));
            }
            if probe_readiness(ops).await.unwrap_or(false) {
                return Ok(());
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        self.stop()?;
        Err(self.failure(&format!("not ready within {READY_TIMEOUT:?}")))
    }

    pub fn terminate(&self) -> Result<(), Box<dyn Error>> {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()?;
        if status.success() {
            return Ok(());
        }
        Err(format!("kill -TERM failed with {status}").into())
    }

    pub async fn wait_for_exit(&mut self, limit: Duration) -> Result<ExitStatus, Box<dyn Error>> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        self.stop()?;
        Err(self.failure(&format!("did not exit within {limit:?} after SIGTERM")))
    }

    /// Returns an error naming `reason` that carries the captured stderr.
    pub fn failure(&mut self, reason: &str) -> Box<dyn Error> {
        if self.child.try_wait().ok().flatten().is_some()
            && let Some(reader) = self.reader.take()
        {
            let _ = reader.join();
        }
        let stderr = self
            .stderr
            .lock()
            .map(|buffer| buffer.clone())
            .unwrap_or_default();
        Box::new(ChildFailure(format!(
            "child {reason}\n--- child stderr ---\n{stderr}--- end child stderr ---"
        )))
    }

    fn stop(&mut self) -> std::io::Result<()> {
        self.child.kill()?;
        self.child.wait().map(drop)
    }
}

/// Debug-formats without escaping, so a failing test prints stderr line by line.
struct ChildFailure(String);

impl fmt::Debug for ChildFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for ChildFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ChildFailure {}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.stop();
        }
    }
}
{% endif -%}
