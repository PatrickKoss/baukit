use std::{collections::BTreeMap, ffi::OsStr, path::Path, time::SystemTime};

/// Returns Docker labels that identify a test container's owning process.
///
/// Includes `dev.baukit.test=true`, the process ID, and the start time in Unix
/// seconds. Includes the executable's file name and the runtime
/// `CARGO_MANIFEST_DIR` when available. Pass these labels to
/// `testcontainers::ImageExt::with_labels` for containers started outside the
/// Baukit fixtures. `scripts/reap-test-containers.sh` uses them to find orphans
/// on the local Linux Docker host.
#[must_use]
pub fn container_labels() -> BTreeMap<String, String> {
    let executable = std::env::current_exe().ok();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok();
    let started = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    labels_for_process(std::process::id(), executable.as_deref(), manifest, started)
}

fn labels_for_process(
    pid: u32,
    executable: Option<&Path>,
    manifest: Option<String>,
    started: u64,
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
    labels
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::SystemTime};

    use super::{container_labels, labels_for_process};

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
        let labels = labels_for_process(42, None, None, 123);
        assert_eq!(labels.len(), 3);
        assert_eq!(labels["dev.baukit.test"], "true");
        assert_eq!(labels["dev.baukit.test.pid"], "42");
        assert_eq!(labels["dev.baukit.test.started-at"], "123");
        assert!(!labels.contains_key("dev.baukit.test.binary"));
        assert!(!labels.contains_key("dev.baukit.test.manifest-dir"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_binary_is_omitted_to_avoid_false_pid_reuse() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

        let executable = Path::new(OsStr::from_bytes(b"/target/test-\xff"));
        let labels = labels_for_process(42, Some(executable), None, 123);
        assert!(!labels.contains_key("dev.baukit.test.binary"));
    }

    #[test]
    fn binary_is_a_file_name_and_manifest_is_the_runtime_value() {
        let labels = labels_for_process(
            42,
            Some(Path::new("/worktree/target/debug/deps/test-binary")),
            Some("/other/worktree/backend".to_owned()),
            123,
        );
        assert_eq!(labels["dev.baukit.test.binary"], "test-binary");
        assert_eq!(
            labels["dev.baukit.test.manifest-dir"],
            "/other/worktree/backend"
        );
        assert_eq!(labels.len(), 5);
    }
}
