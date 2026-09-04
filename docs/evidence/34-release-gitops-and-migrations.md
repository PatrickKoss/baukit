# 34. Release, GitOps, and migration compatibility

## Source product files

- `/home/patrick/projects/leitbild/docs/releasing.md`
- `/home/patrick/projects/leitbild/docs/baukit-replay-assessment.md`
- `/home/patrick/projects/leitbild/scripts/release.sh`
- `/home/patrick/projects/leitbild/scripts/promote.sh`
- `/home/patrick/projects/leitbild/scripts/prune-ghcr.sh`
- `/home/patrick/projects/leitbild/scripts/set-release-tag.py`
- `/home/patrick/projects/leitbild/scripts/check-expand-contract.sh`
- `/home/patrick/projects/leitbild/scripts/render-gitops.py`
- `/home/patrick/projects/leitbild/deploy/gitops/testing/release.yaml`
- `/home/patrick/projects/leitbild/deploy/gitops/production/release.yaml`
- `scripts/release-train.sh`
- `scripts/check-version-coherence.py`
- `docs/releasing.md`
- `deploy/platform/README.md`
- `deploy/platform/validate.sh`
- `deploy/chart/baukit-app/README.md`
- `templates/common/__strict__/__backend__/scripts/check-migrations-immutable.sh`

## Observed failure or repeated glue

Leitbild repeats a useful invariant across shell, Python, YAML comments, and release documentation: every process image comes from one source commit, testing changes only after all pushes succeed, and production promotes the tested artifacts without rebuilding. The scripts hard-code one repository, branch, registry owner, cluster, process set, file layout, and retention count. The pin editor and migration gate use regular expressions where parsed identity and reviewed semantics are needed.

## Baukit owner

Deployment documentation should own the release-manifest and expand-and-contract contracts. A later Baukit CLI command or deployment validator may parse the manifest and emit a patch. Baukit must not own environment mutation, registry access, Git hosting, cluster access, or product migration decisions.

## Public types and errors

The candidate contract names `ReleaseManifestV1Alpha1`, `ReleaseSource`, `ReleaseProcess`, `ReleaseTarget`, `GitOpsLocation`, `ValuesTarget`, `YamlPath`, `ReleaseValidationPlan`, and `ReleaseChange`. Stable errors are `invalid_manifest`, `wrong_source_revision`, `wrong_gitops_repository`, `wrong_gitops_revision`, `unsafe_target_path`, `document_not_found`, `document_not_unique`, `value_not_scalar`, `unexpected_repository`, `mutable_pin`, `mixed_source_revision`, and `dirty_target`. No mutation API is proposed.

## Product-owned inputs

Products supply repository locations and branch policy, processes, image repositories and digests, source revision, target files and document identities, environment names, chart layout, build and push commands, registry credentials, approval, retention, rollback window, migration SQL, lock behavior, backfill policy, compatibility suites, and reviewed exceptions.

## Concurrency, failure, privacy, and cleanup cases

Validation must detect source or GitOps revision drift, a dirty target, changed remote, path traversal, symlink escape, missing or duplicate YAML documents, missing or non-scalar paths, duplicate process targets, stale repository values, mixed source revisions, malformed or mutable pins, and no-op releases. It makes no network, registry, cluster, or Git-host calls and never reads secret values. Remote URLs are redacted before errors. A patch names only declared release values. Concurrent mutation is outside the tool; a future apply step must recheck the expected GitOps commit and manifest digest. The caller owns patch-file cleanup.

Migration verification covers a failed or retried migration, an idempotent interrupted backfill, old and new reads and writes, mixed API and worker versions, rollback after schema expansion, and refusal to contract before the rollback window and review evidence. Private row data, SQL parameters, and secret configuration stay out of errors and logs.

## Supported runtimes

The manifest is operating-system neutral YAML. A future validator should run wherever the Baukit CLI runs and should require only local source and GitOps checkouts. Migration compatibility executes against Baukit's supported PostgreSQL container and the product's released N-1 and candidate N binaries. Kubernetes rendering remains with the existing pinned `kustomize`, `kubeconform`, and Helm workflow.

## Product adoption change

Once a released validator handles Leitbild's manifest and exact YAML targets, Leitbild can delete `scripts/set-release-tag.py` and replace its calls in `scripts/release.sh` and `scripts/promote.sh` with patch validation. Those scripts remain because build, push, promotion, and pull-request actions are product-owned. A later metadata or parser-based migration gate with N/N-1 execution can delete `scripts/check-expand-contract.sh`. Product checks in `scripts/render-gitops.py` remain, except assertions that become Baukit chart conformance tests.

## Throwaway experiments

None. This study used read-only source inspection. No release, registry, Git, cluster, or migration command ran.
