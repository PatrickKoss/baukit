# Run the platform on a local k3d cluster

At the end of this page you have a persistent k3d cluster on your machine that Flux keeps in sync with your own GitOps repository. It survives Docker and host restarts, and one command rebuilds it from scratch. The page is for developers who want a cluster to deploy a product to, or to work on the platform bases. Writing new bases is covered in the [platform bases README](../../deploy/platform/README.md).

## Prerequisites

Install these tools and put them on `PATH`:

- `age`, `docker`, `flux`, `git`, `k3d`, `kubectl`, `sops` and `tar`

`platform-lifecycle.sh` checks for each one and exits with `missing required tools` if any is absent. The Flux version comes from `[tools.flux]` in `deploy/tools.toml`. The k3s image comes from `platform_k3s_image` in `deploy/platform/local-git-http.sh`.

You also need:

- a GitOps repository that already holds a Flux entrypoint at the cluster path you configure
- a SOPS age identity in a file outside any repository, mode 0600, backed up separately
- for GitHub bootstrap, a `GITHUB_TOKEN` in the environment, for example from `gh auth token`

The script never stores the token.

## Write the config file

Settings come from environment variables or from a file passed with `PLATFORM_CONFIG`. The script sources that file as shell, so keep it local, owned by you, and limited to assignments. Do not put the age identity itself in it.

A minimal file for GitHub bootstrap:

```sh
PLATFORM_CLUSTER_NAME=local
PLATFORM_GITOPS_URL=ssh://git@github.com/ORGANIZATION/REPOSITORY
PLATFORM_GITOPS_PATH=clusters/local
PLATFORM_GITHUB_OWNER=ORGANIZATION
PLATFORM_GITHUB_REPOSITORY=REPOSITORY
PLATFORM_AGE_KEY_FILE="$HOME/.config/sops/age/platform.txt"
```

The keys you are most likely to set:

| Variable | Default | Meaning |
|---|---|---|
| `PLATFORM_CLUSTER_NAME` | required | k3d cluster name: lowercase letters, digits, dots and hyphens |
| `PLATFORM_GITOPS_URL` | required for `up` | URL the cluster's Git source uses |
| `PLATFORM_GITOPS_PATH` | required for `up` | cluster entrypoint inside the repository |
| `PLATFORM_GITOPS_BRANCH` | `main` | branch Flux reconciles |
| `PLATFORM_GITOPS_AUTH_MODE` | `github` | `github`, or `ssh-key` with `PLATFORM_GITOPS_PRIVATE_KEY_FILE` |
| `PLATFORM_GITHUB_OWNER_TYPE` | `organization` | `organization` or `personal` |
| `PLATFORM_AGE_KEY_FILE` | required for `up` | installed as Secret `flux-system/sops-age` |
| `PLATFORM_BAUKIT_SOURCE_MODE` | `github-tag` | `github-tag`, or `local-snapshot` with `PLATFORM_BAUKIT_CHECKOUT` |
| `PLATFORM_API_PORT` | `random` | Kubernetes API port on `127.0.0.1` |
| `PLATFORM_HTTP_PORT` | `80` | HTTP ingress port on `127.0.0.1` |
| `PLATFORM_HTTPS_PORT` | `443` | HTTPS ingress port on `127.0.0.1` |
| `PLATFORM_SUBNET` | `10.89.1.0/24` | Docker network of a new cluster |

The defaults are read in `deploy/platform/platform-lifecycle.sh`. The [configuration contract](../../deploy/platform/README.md#configuration-contract) lists the rest, including the snapshot port, state directory and extra registry mirrors.

`local-snapshot` mode serves a copy of your baukit checkout to the cluster instead of a released tag. The script copies the files Git would commit, so ignored files such as `.env` stay out. Your GitOps overlay then points `GitRepository/baukit` at `http://host.k3d.internal:9418/baukit.git`. Use it to test platform changes before a release.

## Start the cluster

```sh
make platform-up PLATFORM_CONFIG=/path/to/local.env
```

The Makefile passes the file to `deploy/platform/platform-lifecycle.sh --config FILE up`. `up` does this:

1. creates the k3d cluster, or starts it if it exists
2. refreshes the local snapshot when `local-snapshot` mode is on
3. sets `restart=unless-stopped` on every cluster and snapshot container
4. installs the age Secret
5. bootstraps Flux and the Git source
6. waits up to 45 minutes each for every Kustomization and HelmRelease to become Ready
7. prints the health report

Rerunning `up` is safe. It converges the existing cluster.

## Check it

```sh
make platform-status PLATFORM_CONFIG=/path/to/local.env
```

`status` prints the k3d cluster, each container with its restart policy, the Flux Git sources, Kustomizations and HelmReleases, every pod that is not Running or Succeeded, and where the secrets live. It never prints a secret value.

## Stop it

```sh
make platform-down PLATFORM_CONFIG=/path/to/local.env
```

`down` stops the k3d and snapshot containers. Volumes, the Kubernetes datastore and snapshot state stay for the next `up`. A stopped cluster stays stopped across Docker restarts until you run `up`.

Two more targets remove state:

- `make platform-nuke` deletes the cluster, its labelled volumes and network, the snapshot container and image, and the state directory. Your GitOps repository and age file stay untouched.
- `make platform-recreate` runs `nuke` and then `up`. Use it to prove the cluster rebuilds from Git alone.

## What runs

The cluster runs whatever your GitOps entrypoint composes from the bases in `deploy/platform/`. A typical local composition, as in `deploy/platform/overlays/local`, has the cluster baseline, cert-manager with a self-signed issuer, Traefik, CloudNativePG with one PostgreSQL cluster, the observability stack, and RustFS for Loki logs and PostgreSQL backups. It runs no Keycloak. Products verify tokens against their own compose Keycloak on the host.

Ingress, the API server and the snapshot server bind to loopback or the k3d network gateway, never to every interface.

## Footprint

[local-footprint.md](../../deploy/platform/local-footprint.md) has the measurements and how they were taken. The short version: the idle local platform with one product used about 6.3 GiB of memory at node level. A single-node testing cluster should get at least 4 vCPU and 16 GiB. Falco and the optional Keycloak base add their own amounts, listed on that page.

## Platform e2e suites

`make platform-e2e` runs `deploy/platform/e2e/run.sh`. It builds a throwaway k3d cluster from a snapshot of your working tree, runs the chainsaw suites in `deploy/platform/e2e/tests/`, and deletes the cluster on exit. It does not touch your lifecycle cluster or your default kubeconfig. Pick suites with `--suite NAME` and keep the cluster with `--keep`. The [e2e README](../../deploy/platform/e2e/README.md) has the flags and the locks it takes.

## Reference

- [platform bases README](../../deploy/platform/README.md)
- [local footprint](../../deploy/platform/local-footprint.md)
- [platform e2e README](../../deploy/platform/e2e/README.md)
- [platform CHANGELOG](../../deploy/platform/CHANGELOG.md)
- [pinned deploy tools](../../deploy/tools.toml)
- [deploying guide](deploying.md)
- [observability guide](observability.md)
- [ADR 0030: deployment platform](../adr/0030-deployment-platform.md)
