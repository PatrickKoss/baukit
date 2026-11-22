# GitOps environments

Flux applies `testing/` and `production/` as the tenant's `flux-reconciler`, which holds `admin` in
the tenant namespace only. Both overlays share `base/`, which holds the HelmRelease of the baukit
chart, its tenant-local `GitRepository/baukit` source, the IngressRoute and a headers Middleware.
TLS comes from the platform wildcard in Traefik's default TLSStore, so no route names a
Certificate. The route still lists `tls.domains` for `{{ context.gitops.domain }}`, because Flux's
server-side apply turns an empty `tls: {}` into `null`, which the IngressRoute CRD rejects. The tenant itself (namespace, Flux registration, database, age identity) lives in the
platform repository.

| Environment | Namespace | Host |
|---|---|---|
| testing | `{{ context.app_name }}-testing` | `{{ context.app_name }}-testing.{{ context.gitops.domain }}` |
| production | `{{ context.app_name }}-production` | `{{ context.app_name }}.{{ context.gitops.domain }}` |

Both run one replica without an HPA and use their own database through `{{ context.app_name }}-runtime`.
{% if context.auth_enabled %}The API trusts `{{ context.gitops.identity_url }}/realms/{{ context.gitops.realm }}`{% if context.auth_oidc %}
with the audience `{{ context.app_name }}-testing` or `{{ context.app_name }}-production`, one realm client
per environment{% endif %}.
{% endif %}
The domain, registry, identity URL and realm come from `[gitops]` in `baukit.toml`
(`domain`, `registry`, optional `identity_url` and `realm`). Change them there and regenerate, or
edit these files directly; they belong to the product.

## Secrets

`*.enc.yaml` files ship as plaintext placeholders. Put the tenant's age recipients into
`.sops.yaml`, fill in the values, and encrypt each file in place:

```sh
sops --config deploy/gitops/.sops.yaml --encrypt --in-place deploy/gitops/testing/runtime-secret.enc.yaml
```

`scripts/release.sh` and `scripts/promote.sh` refuse to run while any `*.enc.yaml` lacks sops
metadata.

## Releases

`.github/workflows/release.yml` runs `scripts/release.sh` on every push to `main` outside
`deploy/gitops/`. It builds the `api`, `migrate`{% if context.worker %} and `worker`{% endif %} images, pushes
`{{ context.gitops.registry }}/{{ context.app_name }}-<process>:<sha12>`, signs each by digest with cosign and
commits the `<sha12>@sha256:<digest>` pins to `testing/release.yaml`.
{% if context.gitops.lab_registry %}With `lab_registry` set in `[gitops]`, the release then copies each image and its signature by digest with `cosign copy` to
`{{ context.gitops.lab_registry }}/{{ context.app_name }}-<process>:<sha12>`. The digest does not change, so
the pins and signatures hold in both registries. Both overlays pull from the lab registry, so they
ship no GHCR pull secret; give the lab cluster pull access to that registry outside this repository.
{% endif %}`.github/workflows/promote.yml` (manual) verifies those signatures with `deploy/cosign.pub` and
copies the pins to `production/release.yaml`. Run either locally with `--dry-run` to see the
commands without building, pushing or signing.

An existing product adds these files with `baukit generate gitops`, which writes only missing files.

Repository setup:

- GitHub environment `release`, limited to `main`, with secrets `COSIGN_PRIVATE_KEY`,
  `COSIGN_PASSWORD` and `GITOPS_DEPLOY_KEY`.
- GitHub environment `production` with a required reviewer and the secret `GITOPS_DEPLOY_KEY`.
- A deploy key with write access whose private half is `GITOPS_DEPLOY_KEY`, allowed to bypass the
  branch protection on `main`.
- `deploy/cosign.pub`, the public half of the cosign key.
{% if context.gitops.lab_registry %}- Repository variable `LAB_REGISTRY_USER` and secret `LAB_REGISTRY_TOKEN` in the `release`
  environment, a push credential for the lab registry.
{% endif %}- Optional repository variables `BAUKIT_RUNS_ON` and `BAUKIT_GATE_RUNS_ON`, each a JSON runner
  label array such as `["self-hosted","lab","x64"]`. Unset, jobs run on `ubuntu-latest`.
