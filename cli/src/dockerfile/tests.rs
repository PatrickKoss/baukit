use super::*;

fn manifest(image: &str) -> Result<Manifest> {
    let source = format!(
        r#"
schema_version = 1
template_version = "0.10.8"
[app]
name = "test-product"
[capabilities]
backend = true
mobile = false
web = false
[dependencies.baukit]
source = "registry"
version = "0.10.8"
[openapi]
schema = "backend/openapi.json"
[backend.image]
{image}
"#
    );
    Ok(toml::from_str(&source)?)
}

fn assert_image_pin(output: &str, key: &str, image: &str) {
    let line = output
        .lines()
        .find(|line| line.starts_with(&format!("ARG {key}=")))
        .expect("base image argument");
    assert!(line.starts_with(&format!("ARG {key}={image}@sha256:")));
    let digest = line.rsplit(':').next().expect("digest");
    assert_eq!(digest.len(), 64);
    assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn defaults_and_worker_follow_capabilities() -> Result<()> {
    let mut manifest = manifest("")?;
    let output = render_dockerfile(&manifest)?;
    assert!(output.starts_with("# syntax=docker/dockerfile:1.28.0\n"));
    assert!(output.ends_with("ENTRYPOINT [\"/app/migrate\"]\n"));
    assert!(output.contains("ARG CARGO_BUILD_JOBS\n"));
    assert!(output.contains("ARG CARGO_TARGET_CACHE_PREFIX=test-product\n"));
    assert!(output.contains("id=${CARGO_TARGET_CACHE_PREFIX}-target,"));
    assert!(output.contains("id=test-product-cargo-registry,"));
    assert!(!output.contains("ENV CARGO_BUILD_JOBS"));
    assert!(output.contains("SQLX_OFFLINE=true"));
    assert!(output.contains("--bin api --bin migrate \\\n"));
    assert!(output.contains("COPY --from=builder /workspace/migrations /workspace/migrations"));
    assert!(output.contains("/workspace/crates/test-product-bin/Cargo.toml"));
    assert!(!output.contains("AS worker"));
    for (key, image) in [
        ("RUST_IMAGE", "rust:1.99.0-trixie"),
        ("RUNTIME_IMAGE", "gcr.io/distroless/cc-debian13:nonroot"),
    ] {
        assert_image_pin(&output, key, image);
    }
    assert!(!output.contains("PACKAGES_RUNTIME_IMAGE"));
    manifest
        .capabilities
        .components
        .insert(crate::Component::Worker);
    let output = render_dockerfile(&manifest)?;
    assert!(output.contains("--bin api --bin migrate --bin worker"));
    assert!(output.contains("FROM ${RUNTIME_IMAGE} AS worker"));
    assert!(!toml::to_string(&manifest)?.contains("[backend"));
    Ok(())
}

#[test]
fn every_option_renders_from_the_manifest() -> Result<()> {
    let manifest = manifest(
        r#"
binaries = ["api", "migrate", "seed"]
bin_crate = "custom-bin"
backend_context = "backend"
cargo_build_jobs = 6
apt_packages = ["clang", "cmake", "libclang-dev", "pkg-config"]
build_inputs = [{ source = "content", destination = "/content/" }]
pre_build = [{ command = ["cargo", "run", "--locked", "--release", "-p", "compiler", "--", "build", "/content", "--out", "/generated/content"], outputs = ["/generated/content"] }]
runtime_files = [{ stage = "api", source = "/generated/content", destination = "/app/content" }, { stage = "migrate", source = "crates/postgres/Cargo.toml", destination = "/workspace/crates/postgres/Cargo.toml" }]
runtime_binaries = [{ stage = "migrate", binary = "seed" }]
writable_directories = [{ stage = "api", path = "/app/var/artifacts" }]
"#,
    )?;
    let output = render_dockerfile(&manifest)?;
    for expected in [
        "ARG BACKEND_CONTEXT=backend",
        "ARG CARGO_BUILD_JOBS=6",
        "ARG BAUKIT_CONTEXT=.",
        "ARG BAUKIT_DESTINATION=/tmp/unused-baukit-context",
        "ARG LIMITS_FILE=limits.json",
        "ARG GIT_COMMIT=unknown",
        "apt-get install --no-install-recommends --yes clang cmake libclang-dev pkg-config",
        "COPY [\"content\", \"/content/\"]",
        "'cargo' 'run' '--locked' '--release' '-p' 'compiler' '--' 'build' '/content' '--out' '/generated/content' \\\n    && cargo build",
        "--bin api --bin migrate --bin seed",
        "-p custom-bin",
        "COPY --from=builder [\"/generated/content\", \"/app/content\"]",
        "COPY --from=builder [\"/workspace/crates/postgres/Cargo.toml\", \"/workspace/crates/postgres/Cargo.toml\"]",
        "COPY --from=builder /out/seed /app/seed",
        "RUN mkdir -p /out/directories/0\n",
        "COPY --from=builder --chown=65532:65532 /out/directories/0 /app/var/artifacts",
        "FROM ${RUNTIME_IMAGE} AS seed",
        "ENTRYPOINT [\"/app/seed\"]",
    ] {
        assert!(output.contains(expected), "missing {expected}\n{output}");
    }
    assert!(!output.contains("AS worker"));
    Ok(())
}

#[test]
fn runtime_packages_apply_only_to_the_selected_stage() -> Result<()> {
    let manifest = manifest(
        r#"
binaries = ["api", "migrate", "worker"]
writable_directories = [{stage = "worker", path = "/app/reports"}]
[[backend.image.runtime_packages]]
stage = "worker"
packages = ["git", "libssl3t64"]
"#,
    )?;
    let output = render_dockerfile(&manifest)?;
    assert_image_pin(&output, "PACKAGES_RUNTIME_IMAGE", "debian:trixie-slim");
    let (other_stages, worker) = output
        .split_once("FROM ${PACKAGES_RUNTIME_IMAGE} AS worker\n")
        .expect("Debian worker stage");
    assert!(other_stages.contains("FROM ${RUNTIME_IMAGE} AS api\n"));
    assert!(other_stages.contains("FROM ${RUNTIME_IMAGE} AS migrate\n"));
    assert!(!other_stages.contains("apt-get"));
    assert!(!other_stages.contains("USER "));
    assert!(worker.starts_with(
            "RUN apt-get update \\\n    && apt-get install --no-install-recommends --yes ca-certificates git libssl3t64 \\\n    && rm -rf /var/lib/apt/lists/*\nUSER 65532:65532\nWORKDIR /app\n"
        ));
    assert!(worker.contains("COPY --from=builder /out/worker /app/worker\n"));
    assert!(
        worker
            .contains("COPY --from=builder --chown=65532:65532 /out/directories/0 /app/reports\n")
    );
    assert!(output.ends_with("ENTRYPOINT [\"/app/worker\"]\n"));
    Ok(())
}

#[test]
fn builder_packages_do_not_add_a_packages_runtime_argument() -> Result<()> {
    let output = render_dockerfile(&manifest("apt_packages = ['git']")?)?;
    assert!(output.contains("apt-get install --no-install-recommends --yes git"));
    assert!(!output.contains("PACKAGES_RUNTIME_IMAGE"));
    Ok(())
}

fn rendered_stage<'a>(output: &'a str, name: &str) -> &'a str {
    output
        .split("\nFROM ")
        .find(|stage| {
            stage
                .lines()
                .next()
                .is_some_and(|line| line.ends_with(&format!(" AS {name}")))
        })
        .expect("rendered stage")
        .trim_end()
}

#[test]
fn variants_build_all_binaries_with_separate_target_caches() -> Result<()> {
    let manifest = manifest(
        r#"
binaries = ["api", "migrate", "worker"]
bin_crate = "custom-bin"
build_inputs = [{source = "content", destination = "/content"}]
pre_build = [{command = ["true"]}]
writable_directories = [{stage = "worker", path = "/app/reports"}]
[[backend.image.variants]]
name = "smoke"
features = ["stack-fixtures", "dep/feature_1-2+3.4"]
"#,
    )?;
    let output = render_dockerfile(&manifest)?;
    let base = rendered_stage(&output, "builder");
    let variant = rendered_stage(&output, "builder-smoke");
    assert_eq!(
            variant,
            base.replace(" AS builder\n", " AS builder-smoke\n")
                .replace("id=${CARGO_TARGET_CACHE_PREFIX}-target,", "id=${CARGO_TARGET_CACHE_PREFIX}-target-smoke,")
                .replace(
                    "--bin api --bin migrate --bin worker ",
                    "--bin api --bin migrate --bin worker --features stack-fixtures,dep/feature_1-2+3.4 "
                )
        );
    assert!(!base.contains("--features"));
    assert!(variant.contains("'true' \\\n    && cargo build --locked --release -p custom-bin"));
    assert_eq!(output.matches("&& cp ").count(), 2);
    assert_eq!(output.matches("id=test-product-cargo-registry,").count(), 2);
    assert_eq!(
        output
            .matches("ARG CARGO_TARGET_CACHE_PREFIX=test-product\n")
            .count(),
        2
    );
    assert!(base.contains("id=${CARGO_TARGET_CACHE_PREFIX}-target,"));
    assert!(variant.contains("id=${CARGO_TARGET_CACHE_PREFIX}-target-smoke,"));
    assert_eq!(output.matches("cargo build ").count(), 2);
    assert!(!output.contains("PACKAGES_RUNTIME_IMAGE"));
    for binary in ["api", "migrate", "worker"] {
        let stage = rendered_stage(&output, &format!("{binary}-smoke"));
        assert!(stage.contains(&format!(
            "COPY --from=builder-smoke /out/{binary} /app/{binary}"
        )));
        assert_eq!(
            stage,
            rendered_stage(&output, binary)
                .replacen(
                    &format!(" AS {binary}\n"),
                    &format!(" AS {binary}-smoke\n"),
                    1
                )
                .replace("--from=builder ", "--from=builder-smoke ")
        );
    }
    assert!(output.find(" AS worker\n") < output.find(" AS api-smoke\n"));
    assert!(output.ends_with("ENTRYPOINT [\"/app/worker\"]\n"));
    Ok(())
}

#[test]
fn variant_packages_merge_with_base_and_keep_runtime_rules() -> Result<()> {
    let manifest = manifest(&format!(
        r#"
binaries = ["api", "migrate", "worker", "seed"]
build_inputs = [{{source = "content", destination = "/content"}}]
pre_build = [{{command = ["true"], outputs = ["/generated"]}}]
runtime_files = [{{stage = "api", source = "/generated/file", destination = "/app/file"}}]
runtime_binaries = [{{stage = "migrate", binary = "seed"}}]
writable_directories = [{{stage = "worker", path = "/app/reports"}}]
downloads = [{{stage = "worker", url = "https://example.com/tool.tar.gz", archive_sha256 = "{}", binary = "tool", binary_sha256 = "{}", destination = "/app/tool"}}]
runtime_packages = [{{stage = "worker", packages = ["git"]}}]
[[backend.image.variants]]
name = "smoke"
features = ["stack-fixtures"]
runtime_packages = [{{stage = "api", packages = ["socat"]}}, {{stage = "worker", packages = ["git", "socat", "socat"]}}]
"#,
        "a".repeat(64),
        "b".repeat(64)
    ))?;
    let output = render_dockerfile(&manifest)?;
    let worker = rendered_stage(&output, "worker");
    assert!(worker.starts_with("${PACKAGES_RUNTIME_IMAGE} AS worker\n"));
    assert!(worker.contains("--yes ca-certificates git \\\n"));
    let variant = rendered_stage(&output, "worker-smoke");
    assert!(variant.starts_with("${PACKAGES_RUNTIME_IMAGE} AS worker-smoke\n"));
    assert!(variant.contains("--yes ca-certificates git socat \\\n"));
    assert!(variant.contains("USER 65532:65532\n"));
    assert!(
        variant.contains(
            "COPY --from=builder-smoke --chown=65532:65532 /out/directories/0 /app/reports"
        )
    );
    assert!(
        variant
            .contains("COPY --from=baukit-download-0 --chown=0:0 --chmod=0555 /out/tool /app/tool")
    );
    assert!(rendered_stage(&output, "api").starts_with("${RUNTIME_IMAGE} AS api\n"));
    let api = rendered_stage(&output, "api-smoke");
    assert!(api.starts_with("${PACKAGES_RUNTIME_IMAGE} AS api-smoke\n"));
    assert!(api.contains("--yes ca-certificates socat \\\n"));
    assert!(api.contains("COPY --from=builder-smoke [\"/generated/file\", \"/app/file\"]"));
    let migrate = rendered_stage(&output, "migrate-smoke");
    assert!(migrate.starts_with("${RUNTIME_IMAGE} AS migrate-smoke\n"));
    assert!(migrate.contains("WORKDIR /workspace\n"));
    assert!(
        migrate.contains("COPY --from=builder-smoke /workspace/migrations /workspace/migrations")
    );
    assert!(migrate.contains("COPY --from=builder-smoke /workspace/crates/test-product-bin/Cargo.toml /workspace/crates/test-product-bin/Cargo.toml"));
    assert!(migrate.contains("COPY --from=builder-smoke /out/seed /app/seed"));
    assert_eq!(output.matches(" AS baukit-download-0\n").count(), 1);
    assert_image_pin(&output, "PACKAGES_RUNTIME_IMAGE", "debian:trixie-slim");
    Ok(())
}

#[test]
fn empty_package_entries_select_slim_with_only_certificates() -> Result<()> {
    let manifest = manifest(
        r#"
runtime_packages = [{stage = "api", packages = []}]
[[backend.image.variants]]
name = "smoke"
features = ["stack-fixtures"]
runtime_packages = [{stage = "migrate", packages = []}]
"#,
    )?;
    let output = render_dockerfile(&manifest)?;
    for name in ["api", "api-smoke", "migrate-smoke"] {
        let stage = rendered_stage(&output, name);
        assert!(stage.starts_with(&format!("${{PACKAGES_RUNTIME_IMAGE}} AS {name}\n")));
        assert!(stage.contains("apt-get install --no-install-recommends --yes ca-certificates \\\n    && rm -rf /var/lib/apt/lists/*\nUSER 65532:65532\n"));
    }
    assert!(rendered_stage(&output, "migrate").starts_with("${RUNTIME_IMAGE} AS migrate\n"));
    assert_image_pin(&output, "PACKAGES_RUNTIME_IMAGE", "debian:trixie-slim");
    Ok(())
}

#[test]
fn omitted_variants_preserve_previous_default_bytes() -> Result<()> {
    use sha2::{Digest, Sha256};

    let output = render_dockerfile(&manifest("")?)?;
    let previous = output
            .replace("ARG CARGO_TARGET_CACHE_PREFIX=test-product\n", "")
            .replace("${CARGO_TARGET_CACHE_PREFIX}-target", "test-product-target")
            .replace(
                "    --mount=type=cache,id=test-product-cargo-git,target=/usr/local/cargo/git,sharing=locked \\\n",
                "",
            )
            .replace(
                "target=/usr/local/cargo/registry,sharing=locked",
                "target=/usr/local/cargo/registry",
            );
    assert_eq!(
        Sha256::digest(previous.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "6fb68c1e45d368b7b0b43ebeeaa84c4668cb6d3fdfcf7d01ab948922f830b485"
    );
    assert_eq!(output, render_dockerfile(&manifest("variants = []")?)?);
    Ok(())
}

#[test]
fn package_and_variant_combinations_preserve_output_whitespace() -> Result<()> {
    for packages in [
        "[]",
        "[{stage = 'api', packages = []}]",
        "[{stage = 'api', packages = ['git']}, {stage = 'migrate', packages = ['curl']}]",
    ] {
        for variants in [
            "[]",
            "[{name = 'smoke', features = ['stack-fixtures']}]",
            "[{name = 'smoke', features = ['a', 'b'], runtime_packages = [{stage = 'migrate', packages = ['socat']}]}, {name = 'debug-2', features = ['debug'], runtime_packages = [{stage = 'api', packages = []}]}]",
        ] {
            for pre_build in ["[]", "[{command = ['true']}]"] {
                let manifest = manifest(&format!(
                    "runtime_packages = {packages}\nvariants = {variants}\npre_build = {pre_build}\n"
                ))?;
                let output = render_dockerfile(&manifest)?;
                assert!(
                    output.lines().all(|line| line == line.trim_end()),
                    "{output}"
                );
                assert!(
                    output.ends_with("ENTRYPOINT [\"/app/migrate\"]\n"),
                    "{output}"
                );
                assert!(!output.contains("\n\n\n"), "{output}");
                assert_eq!(
                    output.contains("ARG PACKAGES_RUNTIME_IMAGE="),
                    packages != "[]" || variants.contains("runtime_packages")
                );
            }
        }
    }
    Ok(())
}

#[test]
fn rejects_invalid_runtime_packages() -> Result<()> {
    for (entry, expected) in [
        (
            "{stage = 'worker', packages = ['git']}",
            "image stage `worker` is not a declared binary",
        ),
        (
            "{stage = '', packages = []}",
            "image stage `` is not a declared binary",
        ),
        (
            "{stage = 'api', packages = ['']}",
            "invalid runtime apt package",
        ),
        (
            "{stage = 'api', packages = ['--bad']}",
            "invalid runtime apt package",
        ),
        (
            "{stage = 'api', packages = ['git;false']}",
            "invalid runtime apt package",
        ),
        (
            "{stage = 'api', packages = ['git curl']}",
            "invalid runtime apt package",
        ),
        (
            "{stage = 'api', packages = []}, {stage = 'api', packages = ['curl']}",
            "duplicate runtime packages stage `api`",
        ),
    ] {
        for source in [
            format!("runtime_packages = [{entry}]"),
            format!(
                "variants = [{{name = 'smoke', features = ['stack-fixtures'], runtime_packages = [{entry}]}}]"
            ),
        ] {
            let error = render_dockerfile(&manifest(&source)?)
                .expect_err(&source)
                .to_string();
            assert!(error.contains(expected), "{source}: {error}");
        }
    }
    for source in [
        "runtime_packages = [{packages = []}]",
        "variants = [{name = 'smoke', features = ['stack-fixtures'], runtime_packages = [{packages = []}]}]",
    ] {
        assert!(
            manifest(source)
                .expect_err(source)
                .to_string()
                .contains("missing field `stage`")
        );
    }
    Ok(())
}

#[test]
fn rejects_invalid_variants() -> Result<()> {
    for name in [
        "",
        "Smoke",
        "1smoke",
        "smoke_test",
        "smoke.test",
        "smoke/other",
        "smoke;false",
        "smöke",
    ] {
        let source = format!("variants = [{{name = '{name}', features = ['stack-fixtures']}}]");
        assert!(
            render_dockerfile(&manifest(&source)?)
                .expect_err(&source)
                .to_string()
                .contains("invalid image variant name")
        );
    }
    for (source, expected) in [
        (
            "variants = [{features = ['stack-fixtures']}]",
            "invalid image variant name",
        ),
        (
            "variants = [{name = 'smoke'}]",
            "must have at least one feature",
        ),
        (
            "variants = [{name = 'smoke', features = []}]",
            "must have at least one feature",
        ),
        (
            "variants = [{name = 'smoke', features = ['']}]",
            "invalid cargo feature",
        ),
        (
            "variants = [{name = 'smoke', features = ['a,b']}]",
            "invalid cargo feature",
        ),
        (
            "variants = [{name = 'smoke', features = ['a b']}]",
            "invalid cargo feature",
        ),
        (
            "variants = [{name = 'smoke', features = ['a;false']}]",
            "invalid cargo feature",
        ),
        (
            "variants = [{name = 'smoke', features = ['a', 'a']}]",
            "duplicate cargo feature `a`",
        ),
        (
            "variants = [{name = 'smoke', features = ['a']}, {name = 'smoke', features = ['b']}]",
            "duplicate image variant `smoke`",
        ),
        (
            "binaries = ['api', 'builder-smoke']\nvariants = [{name = 'smoke', features = ['a']}]",
            "duplicate image stage `builder-smoke`",
        ),
        (
            "binaries = ['api', 'api-smoke']\nvariants = [{name = 'smoke', features = ['a']}]",
            "duplicate image stage `api-smoke`",
        ),
        (
            "binaries = ['api', 'api-debug']\nvariants = [{name = 'debug-smoke', features = ['a']}, {name = 'smoke', features = ['b']}]",
            "duplicate image stage `api-debug-smoke`",
        ),
    ] {
        let error = render_dockerfile(&manifest(source)?)
            .expect_err(source)
            .to_string();
        assert!(error.contains(expected), "{source}: {error}");
    }
    Ok(())
}

#[test]
fn downloads_verify_archive_and_binary_and_copy_as_root() -> Result<()> {
    let manifest = manifest(&format!(
        r#"
downloads = [{{ stage = "api", url = "https://example.com/tool.tar.gz", archive_sha256 = "{}", binary = "tool", binary_sha256 = "{}", destination = "/app/tool" }}]
"#,
        "a".repeat(64),
        "b".repeat(64)
    ))?;
    let output = render_dockerfile(&manifest)?;
    assert!(output.contains(&format!(
        "ADD --checksum=sha256:{} https://example.com/tool.tar.gz /tmp/release.tar.gz",
        "a".repeat(64)
    )));
    assert!(output.contains("tar -xzf /tmp/release.tar.gz -C /out -- tool"));
    assert!(output.contains(&format!(
        "echo '{}  /out/tool' | sha256sum --check",
        "b".repeat(64)
    )));
    assert!(
        output
            .contains("COPY --from=baukit-download-0 --chown=0:0 --chmod=0555 /out/tool /app/tool")
    );
    Ok(())
}

#[test]
fn pre_build_quotes_arguments_and_accepts_output_children() -> Result<()> {
    let manifest = manifest(
        r#"
pre_build = [{ command = ["printf", "it's $HOME; $(false)"], outputs = ["/generated"] }]
runtime_files = [{ stage = "api", source = "/generated/content/file", destination = "/app/file" }, { stage = "api", source = "/workspace", destination = "/app/workspace" }]
"#,
    )?;
    let output = render_dockerfile(&manifest)?;
    assert!(output.contains("'printf' 'it'\\''s $HOME; $(false)'"));
    assert!(output.contains("COPY --from=builder [\"/workspace\", \"/app/workspace\"]"));
    Ok(())
}

fn download_declaration(field: &str, value: &str) -> String {
    format!(
        r#"downloads = [{{stage = "{}", url = "{}", archive_sha256 = "{}", binary = "{}", binary_sha256 = "{}", destination = "{}"}}]"#,
        if field == "stage" { value } else { "api" },
        if field == "url" {
            value
        } else {
            "https://example.com/tool.tar.gz"
        },
        if field == "archive_sha256" {
            value.to_owned()
        } else {
            "a".repeat(64)
        },
        if field == "binary" { value } else { "tool" },
        if field == "binary_sha256" {
            value.to_owned()
        } else {
            "b".repeat(64)
        },
        if field == "destination" {
            value
        } else {
            "/app/tool"
        }
    )
}

#[test]
fn rejects_each_invalid_image_declaration() -> Result<()> {
    let mut cases = vec![
            ("binaries = []".to_owned(), "must not be empty"),
            ("binaries = [\"api\", \"api\"]".to_owned(), "duplicate"),
            ("binaries = [\"../api\"]".to_owned(), "invalid image"),
            ("binaries = [\"builder\"]".to_owned(), "reserved"),
            ("binaries = [\"baukit-download-0\"]".to_owned(), "reserved"),
            ("bin_crate = \"bad;crate\"".to_owned(), "invalid image"),
            ("backend_context = \"/backend\"".to_owned(), "relative"),
            ("backend_context = \"../backend\"".to_owned(), "without `..`"),
            ("cargo_build_jobs = 0".to_owned(), "positive"),
            ("build_inputs = [{source = \"/content\", destination = \"/content\"}]".to_owned(), "relative"),
            ("build_inputs = [{source = \"../content\", destination = \"/content\"}]".to_owned(), "without `..`"),
            ("build_inputs = [{source = \"content\", destination = \"content\"}]".to_owned(), "absolute"),
            ("build_inputs = [{source = \"content\", destination = \"/../content\"}]".to_owned(), "without `..`"),
            ("pre_build = [{command = []}]".to_owned(), "command must not be empty"),
            ("pre_build = [{command = [\"\"]}]".to_owned(), "command must not be empty"),
            ("pre_build = [{command = [\"line\\nline\"]}]".to_owned(), "newlines or NUL"),
            ("pre_build = [{command = [\"true\"], outputs = [\"../file\"]}]".to_owned(), "without `..`"),
            ("runtime_files = [{stage = \"absent\", source = \"file\", destination = \"/app/file\"}]".to_owned(), "not a declared binary"),
            ("runtime_files = [{stage = \"api\", source = \"/outside/file\", destination = \"/app/file\"}]".to_owned(), "declared pre-build output"),
            ("runtime_files = [{stage = \"api\", source = \"/workspace/../file\", destination = \"/app/file\"}]".to_owned(), "without `..`"),
            ("runtime_files = [{stage = \"api\", source = \"file\", destination = \"file\"}]".to_owned(), "absolute"),
            ("runtime_binaries = [{stage = \"absent\", binary = \"api\"}]".to_owned(), "not a declared binary"),
            ("runtime_binaries = [{stage = \"api\", binary = \"absent\"}]".to_owned(), "not a declared binary"),
            ("writable_directories = [{stage = \"absent\", path = \"/app/files\"}]".to_owned(), "not a declared binary"),
            ("writable_directories = [{stage = \"api\", path = \"files\"}]".to_owned(), "absolute"),
            ("writable_directories = [{stage = \"api\", path = \"/app/../files\"}]".to_owned(), "without `..`"),
            ("apt_packages = [\"--bad\"]".to_owned(), "invalid builder apt"),
            ("apt_packages = [\"clang;false\"]".to_owned(), "invalid builder apt"),
            ("build_inputs = [{source = \"\", destination = \"/app/file\"}]".to_owned(), "must not be empty"),
            ("build_inputs = [{source = \"$FILE\", destination = \"/app/file\"}]".to_owned(), "plain path"),
        ];
    for (field, value, expected) in [
        ("stage", "absent", "not a declared binary"),
        ("url", "http://example.com/tool", "https"),
        ("url", "https:///tool", "https"),
        ("url", "https://user:pass@example.com/tool", "https"),
        ("url", "https://example.com/bad url", "https"),
        ("archive_sha256", "abcd", "64 hexadecimal"),
        ("binary_sha256", &"z".repeat(64), "64 hexadecimal"),
        ("binary", "../tool", "without `..`"),
        ("binary", "/tool", "relative"),
        ("destination", "tool", "absolute"),
    ] {
        cases.push((download_declaration(field, value), expected));
    }
    for (source, expected) in cases {
        let error = render_dockerfile(&manifest(&source)?)
            .expect_err(&source)
            .to_string();
        assert!(error.contains(expected), "{source}: {error}");
    }
    let mut frontend = manifest("cargo_build_jobs = 6")?;
    frontend
        .capabilities
        .components
        .remove(crate::Component::Backend);
    assert!(
        validate(&frontend)
            .expect_err("frontend image")
            .to_string()
            .contains("capabilities.backend")
    );
    Ok(())
}

#[test]
fn rejects_unknown_fields_in_every_image_table() -> Result<()> {
    for source in [
        "typo = true",
        "build_inputs = [{ source = 'file', destination = '/file', typo = true }]",
        "pre_build = [{command = ['true'], typo = true}]",
        "runtime_files = [{stage = 'api', source = 'file', destination = '/file', typo = true}]",
        "runtime_binaries = [{stage = 'api', binary = 'migrate', typo = true}]",
        "runtime_packages = [{stage = 'api', packages = ['git'], typo = true}]",
        "variants = [{name = 'smoke', features = ['stack-fixtures'], typo = true}]",
        "variants = [{name = 'smoke', features = ['stack-fixtures'], runtime_packages = [{stage = 'api', packages = [], typo = true}]}]",
        "writable_directories = [{stage = 'api', path = '/files', typo = true}]",
        "downloads = [{stage = 'api', url = 'https://example.com/tool', archive_sha256 = 'a', binary = 'tool', binary_sha256 = 'b', destination = '/tool', typo = true}]",
    ] {
        assert!(
            manifest(source)
                .expect_err(source)
                .to_string()
                .contains("unknown field")
        );
    }
    assert!(
        toml::from_str::<BackendManifest>("typo = true")
            .expect_err("backend table typo")
            .to_string()
            .contains("unknown field")
    );
    Ok(())
}
