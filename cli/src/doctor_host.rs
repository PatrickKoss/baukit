use std::{
    ffi::OsString,
    fs, io,
    path::Path,
    process::{Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

#[derive(Debug)]
pub(crate) struct DoctorCommandOutput {
    pub(crate) success: bool,
    pub(crate) code: Option<i32>,
    pub(crate) stderr: String,
}

pub(crate) trait DoctorHost {
    fn env_var_os(&self, name: &str) -> Option<OsString>;
    fn is_socket(&self, path: &Path) -> bool;
    fn run_command(
        &self,
        program: &str,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> io::Result<DoctorCommandOutput>;
}

pub(crate) struct SystemDoctorHost;

impl DoctorHost for SystemDoctorHost {
    fn env_var_os(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn is_socket(&self, path: &Path) -> bool {
        is_socket(path)
    }

    fn run_command(
        &self,
        program: &str,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> io::Result<DoctorCommandOutput> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(directory) = current_dir {
            command.current_dir(directory);
        }
        if program == "git" {
            command.env("GIT_TERMINAL_PROMPT", "0").env(
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o IdentityFile=none",
            );
        }
        let output = command.output()?;
        Ok(DoctorCommandOutput {
            success: output.status.success(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

#[cfg(unix)]
pub(crate) fn is_socket(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(not(unix))]
pub(crate) fn is_socket(_path: &Path) -> bool {
    false
}

pub(crate) fn diagnose_ssh_agent(
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> bool {
    let Some(socket) = host
        .env_var_os("SSH_AUTH_SOCK")
        .filter(|value| !value.is_empty())
    else {
        failures.push(
            "SSH_AUTH_SOCK is unset; start an agent with `eval \"$(ssh-agent -s)\"`, then load a key with `ssh-add ~/.ssh/<private-key>`"
                .to_owned(),
        );
        return false;
    };
    if !host.is_socket(Path::new(&socket)) {
        failures.push(
            "SSH_AUTH_SOCK does not point to a socket; restart the agent, then load a key with `ssh-add ~/.ssh/<private-key>`"
                .to_owned(),
        );
        return false;
    }

    let args = vec!["-l".to_owned()];
    match host.run_command("ssh-add", &args, None) {
        Ok(output) if output.success => {
            successes.push("SSH agent is usable and has at least one loaded identity".to_owned());
            true
        }
        Ok(output) if output.code == Some(1) => {
            failures.push(
                "SSH agent has no loaded identities; add one with `ssh-add ~/.ssh/<private-key>`"
                    .to_owned(),
            );
            false
        }
        Ok(output) if output.code == Some(2) => {
            failures.push(
                "SSH agent is unusable; restart it with `eval \"$(ssh-agent -s)\"`, then add a key with `ssh-add ~/.ssh/<private-key>`"
                    .to_owned(),
            );
            false
        }
        Ok(_) => {
            failures.push(
                "could not query the SSH agent; restart it, then add a key with `ssh-add ~/.ssh/<private-key>`"
                    .to_owned(),
            );
            false
        }
        Err(_) => {
            failures.push(
                "could not run `ssh-add`; install an OpenSSH client, start an agent, and load a key"
                    .to_owned(),
            );
            false
        }
    }
}

pub(crate) fn probe_git_dependency(
    host: &dyn DoctorHost,
    git: &str,
    tag: &str,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    let args = ["ls-remote", "--exit-code", "--tags", git, tag]
        .map(str::to_owned)
        .to_vec();
    match host.run_command("git", &args, None) {
        Ok(output) if output.success => successes.push(format!(
            "Baukit git dependency resolves at tag {tag} through the current SSH identity"
        )),
        Ok(_) => failures.push(format!(
            "Baukit git dependency `{git}` tag `{tag}` is not resolvable; check network access, SSH repository access, and the release tag"
        )),
        Err(error) => failures.push(format!(
            "could not run git to resolve Baukit dependency: {error}"
        )),
    }
}

pub(crate) fn has_docker_build_targets(root: &Path) -> bool {
    ["compose.yaml", "Dockerfile", "backend/Dockerfile"]
        .iter()
        .any(|relative| root.join(relative).is_file())
}

pub(crate) fn diagnose_docker(
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> bool {
    let version_args = ["version", "--format", "{{.Server.Version}}"]
        .map(str::to_owned)
        .to_vec();
    match host.run_command("docker", &version_args, None) {
        Ok(output) if output.success => {}
        Ok(_) => {
            failures.push(
                "Docker daemon is unavailable; start Docker before building generated images"
                    .to_owned(),
            );
            return false;
        }
        Err(_) => {
            failures.push(
                "Docker is unavailable; install Docker and start it before building generated images"
                    .to_owned(),
            );
            return false;
        }
    }

    let buildx_args = vec!["buildx".to_owned(), "version".to_owned()];
    match host.run_command("docker", &buildx_args, None) {
        Ok(output) if output.success => {
            successes.push("Docker daemon and BuildKit are available".to_owned());
            true
        }
        Ok(_) | Err(_) => {
            failures.push(
                "Docker BuildKit is unavailable; install or enable Docker Buildx before building generated images"
                    .to_owned(),
            );
            false
        }
    }
}
