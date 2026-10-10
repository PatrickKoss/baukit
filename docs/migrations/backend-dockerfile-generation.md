# Generate backend Dockerfiles from the manifest

Baukit now renders the backend Dockerfile from `baukit.toml`. Use the CLI that
matches the product's Baukit version. Add the image settings below, run
`baukit generate dockerfile`, and review the Dockerfile diff. Commit the
manifest and Dockerfile together. After each Baukit update, regenerate the
file instead of copying the template by hand.

Keep `schema_version = 1`. The new section is optional. Defaults build `api`
and `migrate`, plus `worker` when `capabilities.worker` is true. Each binary
has its own runtime stage and `/app/<binary>` entrypoint. Existing
`docker build --target api`, `--target migrate`, and `--target worker` commands
keep selecting those stages.

| Product | Image fields needed |
| --- | --- |
| Leitbild | `cargo_build_jobs` |
| Hebkit | `backend_context`, `binaries`, `cargo_build_jobs`, `apt_packages` |
| Redemut | `backend_context`, `cargo_build_jobs`, `build_inputs`, `pre_build`, `runtime_files` |
| Eigenruhe | `cargo_build_jobs` |
| Tiefgang | `backend_context`, `cargo_build_jobs`, `build_inputs` |
| Solo Leveling System | `backend_context`, `binaries`, `cargo_build_jobs`, `build_inputs` |
| Runtime Analyzer | `binaries`, `cargo_build_jobs`, `build_inputs`, `runtime_binaries`, `writable_directories`, `downloads` |
| Schlauzug | `cargo_build_jobs`, `build_inputs` |
| AHP | `runtime_packages`, `build_features` |

These examples assume the current `capabilities.worker` declarations. Set it
to true where a product uses worker and relies on the default binary list.
`backend_context` sets a build-arg default. Omit it for a standalone backend
context or keep passing `--build-arg BACKEND_CONTEXT=backend` when building
from the repository root. Extra input sources are always relative to the
actual Docker build context. That context must contain each source.

## Leitbild and Eigenruhe

Both keep the default binary list including worker. Limits are copied through
`LIMITS_FILE`, which defaults to `limits.json`.

```toml
[backend.image]
cargo_build_jobs = 6
```

Eigenruhe's combined `runtime` stage disappears. Update Helm and Compose to
select the `api`, `migrate`, or `worker` image. Set a command override where a
container needs to run a different binary.

## Hebkit

```toml
[backend.image]
backend_context = "backend"
cargo_build_jobs = 6
binaries = ["api", "migrate", "worker", "seed", "healthcheck", "container-entrypoint"]
apt_packages = ["clang", "cmake", "libclang-dev", "pkg-config"]
```

Hebkit currently calls the entrypoint binary `container-entrypoint`; keep that
name until the Cargo target changes. Each listed command gets its own image
stage. The shared runtime containing every command is removed. If the
entrypoint still dispatches to companion commands, declare them through
`runtime_binaries`, or select commands per Compose service and Helm component.
Run migrations through the `baukit-app` chart's pre-install and pre-upgrade
migration hook Job, and in Compose through a `migrate` service that the API
and worker depend on with `condition: service_completed_successfully`. The
chart has no init container support. The named command stages are the
preferred deployment targets.

Move `HEBKIT__HTTP__BIND_ADDRESS`, `HEBKIT__OPS__BIND_ADDRESS`,
`HEBKIT__OPS__PORT`, `HEBKIT__TELEMETRY__LOG_FORMAT`, `RUN_MIGRATIONS` and
`RUST_LOG` into Helm or Compose environment settings. Keep service and
container ports there too. `EXPOSE` is removed.

Baukit supplies digest-pinned Rust, distroless and Debian slim defaults.
Build args can override them; the manifest does not pin base images. BuildKit
cache mounts replace cargo-chef's planner and dependency stages.

## Redemut

```toml
[backend.image]
backend_context = "backend"
cargo_build_jobs = 6
build_inputs = [{ source = "content", destination = "/content/" }]
pre_build = [{ command = ["cargo", "run", "--locked", "--release", "-p", "redemut-content-compiler", "--", "build", "/content", "--out", "/workspace/dist/content"], outputs = ["dist/content"] }]
runtime_files = [
  { stage = "api", source = "dist/content", destination = "/app/dist/content" },
  { stage = "migrate", source = "crates/redemut-postgres/Cargo.toml", destination = "/workspace/crates/redemut-postgres/Cargo.toml" },
]
```

The compiler runs before the bin crate build in the same Cargo cache mount.
The migrate stage keeps the postgres crate's manifest because that crate
resolves migrations relative to its compile-time directory. Baukit also
copies the default migrations and bin crate manifest.

## Tiefgang

```toml
[backend.image]
backend_context = "backend"
cargo_build_jobs = 6
build_inputs = [{ source = "fixtures/suite-events", destination = "/fixtures/suite-events/" }]
```

## Solo Leveling System

```toml
[backend.image]
backend_context = "backend"
cargo_build_jobs = 6
binaries = ["api", "migrate", "worker", "seed", "entrypoint", "healthcheck"]
build_inputs = [
  { source = "fixtures/suite-events/v1/peers.json", destination = "/fixtures/suite-events/v1/peers.json" },
  { source = "fixtures/suite-events/v1/catalog.json", destination = "/fixtures/suite-events/v1/catalog.json" },
]
```

Copy peers and catalog separately. The old `SUITE_PEERS_FILE` and
`SUITE_CATALOG_FILE` image args become manifest sources. If more suite-event
files become build inputs, replace those copies with the Tiefgang directory
copy.

The `entrypoint` stage is its own image. API, migrate and worker invoke their
own binaries directly. Run migrations through the `baukit-app` chart's
pre-install and pre-upgrade migration hook Job, and in Compose through a
`migrate` service that the API and worker depend on with
`condition: service_completed_successfully`. If the product still needs the dispatcher image,
`runtime_binaries` can copy its companion commands into the entrypoint stage.

Move `SOLO_LEVELING_SYSTEM__HTTP__BIND_ADDRESS`,
`SOLO_LEVELING_SYSTEM__HTTP__PORT`, `SOLO_LEVELING_SYSTEM__OPS__BIND_ADDRESS`,
`SOLO_LEVELING_SYSTEM__OPS__PORT` and `RUN_MIGRATIONS` into Helm or Compose.
Ports belong in their service declarations; `EXPOSE` is removed.

## Runtime Analyzer

The manifest's `app.name` is `finops`, so the bin crate defaults to `finops-bin`.

```toml
[backend.image]
cargo_build_jobs = 6
binaries = ["api", "ingest", "migrate", "seed", "worker", "feed-bundle"]
build_inputs = [
  { source = "packages/proto", destination = "/packages/proto/" },
  { source = "fixtures", destination = "/fixtures/" },
]
runtime_binaries = [
  { stage = "migrate", binary = "seed" },
  { stage = "worker", binary = "feed-bundle" },
]
writable_directories = [
  { stage = "api", path = "/app/var/artifacts" },
  { stage = "ingest", path = "/app/var/artifacts" },
  { stage = "worker", path = "/app/var/artifacts" },
  { stage = "worker", path = "/tmp/finops-reports" },
]
downloads = [{ stage = "worker", url = "https://github.com/anchore/grype/releases/download/v0.120.1/grype_0.120.1_linux_amd64.tar.gz", archive_sha256 = "0a9ee97ef5ae2ee953b0a80098105052e846cdbe319a57d808b519c33cd1343d", binary = "grype", binary_sha256 = "d6e3248b0e788b4da7450a9e03d1e72811771cf97de3640a18e6517bf6507eb7", destination = "/app/grype" }]
```

Baukit downloads the tarball with `ADD --checksum`, verifies the extracted
binary's SHA-256, then copies it as root with mode `0555`. This Grype archive
is for Linux amd64. Use an appropriate archive and both checksums when
changing the worker platform. Writable directories belong to nonroot.

The migrator's bin crate manifest and migrations remain automatic. If another
crate resolves runtime migrations, add its manifest with `runtime_files`, as
in Redemut. Builder file sources are relative to `/workspace`, or absolute
under it. A source outside `/workspace` must appear in `pre_build.outputs`.

## Schlauzug

```toml
[backend.image]
cargo_build_jobs = 6
build_inputs = [{ source = "fixtures", destination = "/fixtures/" }]
```

This replaces the old `FIXTURES_CONTEXT` image arg. Set the manifest source to
the fixtures directory relative to the chosen build context.

## AHP

Keep `capabilities.worker = true` or include `worker` in `backend.image.binaries`.

```toml
[[backend.image.runtime_packages]]
stage = "worker"
packages = ["git"]

[[backend.image.build_features]]
binary = "worker"
features = ["smoke"]
```

The worker stage uses the pinned Debian slim base, installs `ca-certificates`
and `git`, removes apt lists, and runs as `65532:65532`. API and migrate keep
the distroless base. `PACKAGES_RUNTIME_IMAGE` can override the Debian base and
appears only when runtime packages are declared. Package lists must be nonempty,
use the same package-name rules as builder `apt_packages`, and name each stage
only once.

Use features that exist in the bin crate. The worker leaves the shared Cargo
build and gets a separate build with `--features smoke` in the same cache
mounts. Multiple features are comma-joined. Each binary entry and its feature
list must be nonempty and cannot contain duplicates. Feature names use letters,
digits, `_`, `-`, `+`, `.` or `/`. If every binary has features, the shared build
is omitted. All binaries still enter `/out/` through one copy.

Both keys accept only names from the image's binary list. Writable directories
use numeric ownership `65532:65532` on both runtime bases.

## Build and check

The renderer retains `BACKEND_CONTEXT`, `BAUKIT_CONTEXT`,
`BAUKIT_DESTINATION`, `LIMITS_FILE` and `GIT_COMMIT`. The last two Baukit args
support a generated product with absolute local Cargo path dependencies when
its image is built from the Baukit repository root. `CARGO_BUILD_JOBS` is the
only jobs arg. It has no default unless the manifest supplies one, and a
build-arg override takes precedence.

Baukit always sets `SQLX_OFFLINE=true`. Generated products commit `.sqlx`
metadata, so builds should use it rather than depend on database access.
Hebkit and Solo Leveling System no longer need separate image settings for it.
Refresh `.sqlx` when queries change.

Run `baukit generate dockerfile --check` in CI. Doctor also fails when the
committed Dockerfile differs and names the file with the regeneration command.
Use `doctor.backend_dockerfile` for a custom output path. The
[CLI reference](../../cli/README.md#backend-image-settings) describes every
field and its validation rules.
