# 34. Release, GitOps, and migration compatibility

## Question and scope

Which parts of Leitbild's release and promotion tooling are neutral enough for Baukit, and where should operational authority remain? This study covers a release manifest, a read-only validator that emits a patch, separate GitOps repositories, and expand-and-contract database rules. It does not authorize image builds, registry writes, Git commits, pushes, pull requests, cluster changes, or migration execution. Source details are in the [evidence record](../evidence/34-release-gitops-and-migrations.md).

## Evidence table

| Product or Baukit source | Files | What it proves | What varies or must not move |
| --- | --- | --- | --- |
| Leitbild release | `/home/patrick/projects/leitbild/scripts/release.sh` and `docs/releasing.md` | Builds four process images from one source commit, pushes all images before changing testing pins, pins the same 12-character source tag, validates the result, and retains rollback images. | Repository and branch, registry owner, package names, process count, local cluster, credentials, stripping, smoke image, retention, commit, and push behavior are product policy. |
| Leitbild promotion | `/home/patrick/projects/leitbild/scripts/promote.sh` | Promotes the exact tag used in testing to production without rebuilding and opens a reviewable branch and pull request. | Branch naming, GitHub CLI use, repository identity, production approval, and pull-request text stay local. |
| Leitbild pin editor | `/home/patrick/projects/leitbild/scripts/set-release-tag.py` and `deploy/gitops/{testing,production}/release.yaml` | Checks a 12-character lowercase SHA and replaces an exact expected number of API, worker, and migration tag lines. | The regular expression relies on comments and product names. It does not parse YAML identity or prove that the file belongs to the intended GitOps checkout. |
| Leitbild migration scan | `/home/patrick/projects/leitbild/scripts/check-expand-contract.sh` | Records a real policy: contract changes wait until promotion and rollback-window expiry. | The scanner matches comments and literals, rejects every added constraint, cannot judge staged operations, and hard-codes grandfathered migration numbers. |
| Leitbild GitOps renderer | `/home/patrick/projects/leitbild/scripts/render-gitops.py` | Renders Kustomize overlays and the Baukit chart, then checks image pins, migration setup, environment wiring, network policy, and progressive delivery. | Resource names, counts, namespaces, egress, canary thresholds, smoke commands, and environment files are product assertions. |
| Baukit release train | [`scripts/release-train.sh`](../../scripts/release-train.sh), [`scripts/check-version-coherence.py`](../../scripts/check-version-coherence.py), and [`docs/releasing.md`](../releasing.md) | Coordinates one Baukit source tag and coherent Rust, TypeScript, template, and chart versions. It requires a clean tree and validates before and after version changes. | It publishes Baukit libraries. It does not build product images or own product environment releases. |
| Baukit platform | [`deploy/platform/README.md`](../../deploy/platform/README.md) and [`deploy/platform/validate.sh`](../../deploy/platform/validate.sh) | Desired state may live in a separate private GitOps repository. Baukit validates local bases, pinned chart sources, rendered Helm releases, and Kubernetes schemas. | Cluster inventory, repository credentials, identity, secrets, domains, and environment values stay in the private repository. |
| Baukit application chart | [`deploy/chart/baukit-app/README.md`](../../deploy/chart/baukit-app/README.md) | Has separate API, worker, and migration images. Migration is a pre-release hook; advisory locking and expand-and-contract compatibility are application duties. Failed migration jobs remain inspectable. | Image repositories, process enablement, migration commands, lock policy, rollout method, and rollback window remain product inputs. |
| Generated migration guard | `templates/common/__strict__/__backend__/scripts/check-migrations-immutable.sh` | Prevents modifying, deleting, or renaming existing migration files between two Git revisions. | It correctly handles migration history immutability but does not claim semantic N/N-1 compatibility. |

The reusable behavior is planning and validation. Leitbild's live release procedure has too many product decisions and external writes to become the first Baukit tool.

## Candidate interface or contract sketch

The release manifest should be versioned, free of credentials, and stored with product release policy. A release instance contains the complete source revision and image digests. YAML paths are arrays, not dotted strings, so keys containing dots stay unambiguous.

```yaml
schemaVersion: baukit.dev/release-manifest/v1alpha1
source:
  repository: https://example.invalid/source.git
  revision: "0123456789abcdef0123456789abcdef01234567"
processes:
  - name: api
    image:
      repository: registry.example.invalid/product-api
      digest: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
      sourceRevision: "0123456789abcdef0123456789abcdef01234567"
targets:
  testing:
    gitops:
      repository: ssh://git@example.invalid/desired-state.git
      branch: main
      root: products/example/testing
    values:
      - file: release.yaml
        document:
          apiVersion: helm.toolkit.fluxcd.io/v2
          kind: HelmRelease
          name: example
        images:
          - process: api
            repositoryPath: [spec, values, api, image, repository]
            pinPath: [spec, values, api, image, digest]
```

The schema requires a distinct process name, a canonical image repository, a `sha256` digest, and a full source revision for each process. Every process revision must equal `source.revision`. A target supplies a GitOps repository location, branch, repository-relative root, one or more values files, an exact Kubernetes document identity, and scalar YAML paths for repository and pin fields. A chart that uses an immutable full-source tag instead of a digest may add a separately named `sourceRevisionTagPath`; it cannot call a shortened SHA or mutable release label an immutable pin.

The first command is read-only:

```text
baukit release validate \
  --manifest release.yaml \
  --source-checkout /path/to/product \
  --gitops-checkout /path/to/desired-state \
  --target testing \
  --expected-gitops-revision <full-commit> \
  [--dry-run | --patch-out release.patch]
```

Inputs are explicit paths to two checkouts, a target name, and the expected current GitOps commit. The validator performs no fetch. It checks that both paths are Git worktrees, the source revision exists in the source checkout, the GitOps remote matches the canonical manifest location, `HEAD` matches the expected revision, and every target path resolves inside the declared root without following a symlink outside it. It parses YAML, selects exactly one document by API version, kind, and name, and requires exactly one scalar at each declared path. It rejects duplicate targets, anchors or aliases at a writable path, a repository mismatch, a missing process, mixed source revisions, mutable tags, malformed digests, an already-dirty target file, and any unlisted file change.

`--dry-run` writes a unified patch to stdout. `--patch-out` writes only the patch artifact and never edits the GitOps checkout. The structured summary contains `manifestVersion`, source and GitOps revisions, target, and a list of `{file, process, repositoryPath, pinPath, before, after}` changes. Stable errors should include `invalid_manifest`, `wrong_source_revision`, `wrong_gitops_repository`, `wrong_gitops_revision`, `unsafe_target_path`, `document_not_found`, `document_not_unique`, `value_not_scalar`, `unexpected_repository`, `mutable_pin`, `mixed_source_revision`, and `dirty_target`. Error values must not include credentials embedded in remote URLs or file contents.

Any later `apply`, push, or pull-request command is a separate, explicitly invoked operation. It must revalidate the GitOps repository, branch, base revision, clean state, exact patch paths, and manifest digest immediately before mutation. Registry authentication, image production, promotion approval, retention, commit messages, and provider integration remain outside the validator.

### Expand-and-contract migration rules

1. Every release declares migrations as `expand`, `backfill`, or `contract`. Existing migration files are immutable. A correction is a new forward migration.
2. An expand migration must leave the resulting schema usable by both application versions N-1 and N. Safe examples are new nullable columns, new tables, and new indexes created with the locking strategy reviewed for the table. Defaults must not force a table rewrite or change old-write meaning without evidence for the supported PostgreSQL version.
3. Renames are staged as add, dual write, backfill, dual read, read-new, stop-old-write, then drop in a later contract release. Type changes use a new column or table unless a PostgreSQL-aware review proves the in-place change safe for both versions.
4. A new required value is introduced nullable, backfilled with an idempotent and resumable job, checked for completeness, then constrained in a contract release. A constraint may be added `NOT VALID` during expand and validated separately when PostgreSQL permits it. Enforcement that rejects N-1 writes waits for contract.
5. Backfills are bounded, restartable, observable, and safe to run more than once. They do not delete old representation while N-1 may read or write it. Product code owns batch size, throttling, private progress data, and stop conditions.
6. Rollback changes application traffic, not schema. The migration job runs once under product-owned locking. API and worker versions N-1 and N must each start and pass read and write compatibility tests against the post-migration schema. If processes roll at different speeds, mixed N/N-1 API-worker combinations are included.
7. A contract migration may run only after N is fully promoted, backfill and dual-write checks pass, no supported binary reads or writes the old representation, and the product's rollback window has expired. The release record names that evidence and the first incompatible application version.
8. Drops, renames, `SET NOT NULL`, type changes, and tightened constraints are contract operations by default. Exceptions need reviewed, time-bounded metadata. A regular-expression match or a migration number is not an exception record.
9. Tests start from the last released schema, run pending migrations with the supported PostgreSQL image, execute N-1 and N application compatibility suites, retry the migration or backfill where promised, and prove a failed migration leaves a diagnosable, recoverable state.

A future gate should consume reviewed sidecar metadata or a PostgreSQL-aware syntax tree. Minimum metadata includes migration identifier and checksum, phase, compatible application range, prerequisites, affected objects, rollback-window evidence for contract, review reference, and exception expiry. The parser can flag suspicious operations for review, but metadata and real N/N-1 execution establish compatibility.

## Required-case coverage

| Required case | Contract coverage |
| --- | --- |
| Process images | The manifest has a distinct process list with canonical repository, digest, and source revision. It supports API, worker, migration, rollout gates, or other product-named processes without a fixed count. |
| Immutable source pins | Full source revisions and `sha256` image digests are required. All process revisions match one release source. Short SHAs may be display values only. |
| Target values files | Each target lists repository-relative files, exact document identity, and array-form YAML paths. The validator rejects zero or multiple matches and any undeclared file. |
| GitOps repository location | The manifest names the canonical repository, branch, and root. The validator accepts a separate checkout and verifies its remote and expected `HEAD` without fetching. |
| Patch output and dry run | Dry run emits a unified patch to stdout. `--patch-out` writes only that artifact. Neither mode edits desired state. A no-op plan is explicit and successful only when every current value already matches. |
| Later mutation | Applying a patch, committing, pushing, or opening a pull request is outside the first tool. A future command needs explicit invocation and must repeat exact repository, branch, revision, cleanliness, and path checks. |
| Product-specific release behavior | Names, branches, registry owners, namespaces, clusters, credentials, process builds, retention, smoke images, approval, and push behavior do not appear in Baukit defaults. |
| Existing Baukit validation | Platform render and schema checks remain in `deploy/platform/validate.sh`; chart guarantees remain chart tests. The release validator checks only release identity and planned value changes. |
| Migration immutability | The existing strict-profile script remains useful and separate. New forward files replace edits to applied migrations. |
| Expand and contract | Rules cover additive schema, staged rename and type changes, nullable-to-required transitions, constraints, idempotent backfills, rollback windows, and explicit contract evidence. |
| SQL analysis | The Leitbild regular-expression scanner is not upstreamed. Future automation uses reviewed metadata or a PostgreSQL-aware parser. |
| N/N-1 execution | Both application versions run against the post-migration schema. Mixed API and worker versions are tested when deployment can produce them. |
| Failure, privacy, and cleanup | Validation fails before patch output on drift or ambiguity, redacts remote credentials, reads no secrets, makes no network or cluster calls, and leaves both checkouts unchanged. The caller owns removal of patch files. |

## Decision

Decision: contract or recipe. Add a versioned release-manifest contract, fixtures for colocated and separate GitOps repositories, and the expand-and-contract rules before writing a command. The smallest implementation after review is a read-only parser and patch generator with no registry, Git host, or cluster client. It needs a generated fixture or second product to prove arbitrary processes and a separate repository. Do not copy Leitbild's shell mutation, pin regex, retention, or SQL regex scanner.

## What stays product-owned

- Source and GitOps repository locations, branch policy, environment names, namespaces, cluster identity, image repositories, build targets, registry authentication, and secrets.
- Which processes ship, promotion order, canary roles and thresholds, smoke commands, approval, rollback window, retention count, rebuild policy, and pull-request workflow.
- Product values, domains, SLOs, encrypted manifests, migration SQL, advisory locks, data backfills, batch limits, object-specific compatibility, and reviewed exceptions.
- Git commits, pushes, pull requests, registry writes, cluster reconciliation, migration execution, and deletion of patch artifacts.
