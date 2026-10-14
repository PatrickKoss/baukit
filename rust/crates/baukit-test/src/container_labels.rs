use std::{collections::BTreeMap, ffi::OsStr, fs, path::Path, time::SystemTime};

/// Returns Docker labels that identify a test container's owning process.
///
/// Includes `dev.baukit.test=true`, the process ID, and the start time in Unix
/// seconds. Adds the executable's file name and runtime `CARGO_MANIFEST_DIR`
/// when available, and the Linux PID namespace and host name when readable.
/// Pass these labels to
/// `testcontainers::ImageExt::with_labels` for containers started outside the
/// Baukit fixtures. `scripts/reap-test-containers.sh` uses them to find orphans
/// on the same Linux host and in the same PID namespace.
#[must_use]
pub fn container_labels() -> BTreeMap<String, String> {
    let executable = std::env::current_exe().ok();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok();
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let namespace = fs::read_link("/proc/self/ns/pid")
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned));
    let hostname = hostname_from_files(
        Path::new("/proc/sys/kernel/hostname"),
        Path::new("/etc/hostname"),
    );
    labels_for_process(
        std::process::id(),
        executable.as_deref(),
        manifest,
        started,
        namespace,
        hostname,
    )
}

fn hostname_from_files(primary: &Path, fallback: &Path) -> Option<String> {
    [primary, fallback].into_iter().find_map(|path| {
        let hostname = fs::read_to_string(path).ok()?;
        let hostname = hostname.trim_end_matches('\n');
        (!hostname.is_empty()).then(|| hostname.to_owned())
    })
}

fn labels_for_process(
    pid: u32,
    executable: Option<&Path>,
    manifest: Option<String>,
    started: u64,
    namespace: Option<String>,
    hostname: Option<String>,
) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::from([
        ("dev.baukit.test".to_owned(), "true".to_owned()),
        ("dev.baukit.test.pid".to_owned(), pid.to_string()),
        ("dev.baukit.test.started-at".to_owned(), started.to_string()),
    ]);
    if let Some(binary) = executable.and_then(Path::file_name).and_then(OsStr::to_str) {
        labels.insert("dev.baukit.test.binary".to_owned(), binary.to_owned());
    }
    if let Some(manifest) = manifest {
        labels.insert("dev.baukit.test.manifest-dir".to_owned(), manifest);
    }
    if let Some(namespace) = namespace {
        labels.insert("dev.baukit.test.pid-ns".to_owned(), namespace);
    }
    if let Some(hostname) = hostname {
        labels.insert("dev.baukit.test.host".to_owned(), hostname);
    }
    labels
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::SystemTime};

    #[cfg(target_os = "linux")]
    use super::hostname_from_files;
    use super::{container_labels, labels_for_process};

    #[cfg(target_os = "linux")]
    #[test]
    fn hostname_uses_the_fallback_when_the_primary_file_is_unreadable() {
        let missing = Path::new("/proc/self/baukit-missing-hostname");
        let fallback = Path::new("/proc/sys/kernel/hostname");
        let expected = std::fs::read_to_string(fallback).expect("fallback hostname");
        assert_eq!(
            hostname_from_files(missing, fallback).as_deref(),
            Some(expected.trim_end_matches('\n'))
        );
        assert_eq!(hostname_from_files(missing, missing), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn labels_identify_the_current_pid_namespace() {
        let namespace = std::fs::read_link("/proc/self/ns/pid").expect("PID namespace");
        assert_eq!(
            container_labels()["dev.baukit.test.pid-ns"],
            namespace.to_str().expect("namespace text")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn labels_identify_the_current_host() {
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
            .expect("hostname");
        assert_eq!(
            container_labels()["dev.baukit.test.host"],
            hostname.trim_end_matches('\n')
        );
    }

    #[test]
    fn labels_identify_the_current_test_process() {
        let before = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("unix time")
            .as_secs();
        let labels = container_labels();
        let after = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("unix time")
            .as_secs();
        assert_eq!(labels["dev.baukit.test"], "true");
        assert_eq!(
            labels["dev.baukit.test.pid"],
            std::process::id().to_string()
        );
        let executable = std::env::current_exe().expect("test executable");
        assert_eq!(
            labels["dev.baukit.test.binary"],
            executable
                .file_name()
                .expect("binary name")
                .to_string_lossy()
        );
        assert_eq!(
            labels.get("dev.baukit.test.manifest-dir"),
            std::env::var("CARGO_MANIFEST_DIR").ok().as_ref()
        );
        let started: u64 = labels["dev.baukit.test.started-at"]
            .parse()
            .expect("unix seconds");
        assert!((before..=after).contains(&started));
    }

    #[test]
    fn optional_labels_are_omitted_when_unavailable() {
        let labels = labels_for_process(42, None, None, 123, None, None);
        assert_eq!(labels.len(), 3);
        assert_eq!(labels["dev.baukit.test"], "true");
        assert_eq!(labels["dev.baukit.test.pid"], "42");
        assert_eq!(labels["dev.baukit.test.started-at"], "123");
        assert!(!labels.contains_key("dev.baukit.test.binary"));
        assert!(!labels.contains_key("dev.baukit.test.manifest-dir"));
        assert!(!labels.contains_key("dev.baukit.test.pid-ns"));
        assert!(!labels.contains_key("dev.baukit.test.host"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_binary_is_omitted_to_avoid_false_pid_reuse() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let executable = Path::new(OsStr::from_bytes(b"/target/test-\xff"));
        let labels = labels_for_process(42, Some(executable), None, 123, None, None);
        assert!(!labels.contains_key("dev.baukit.test.binary"));
    }

    #[test]
    fn binary_is_a_file_name_and_manifest_is_the_runtime_value() {
        let labels = labels_for_process(
            42,
            Some(Path::new("/worktree/target/debug/deps/test-binary")),
            Some("/other/worktree/backend".to_owned()),
            123,
            None,
            None,
        );
        assert_eq!(labels["dev.baukit.test.binary"], "test-binary");
        assert_eq!(
            labels["dev.baukit.test.manifest-dir"],
            "/other/worktree/backend"
        );
        assert_eq!(labels.len(), 5);
    }
}
