# shellcheck shell=bash
# Sourced by platform-up.sh and platform-lifecycle.sh. Flux multi-tenancy lockdown:
# components/flux-lockdown holds the patches, these helpers render and verify them.

flux_lockdown_components=source-controller,kustomize-controller,helm-controller,notification-controller
flux_lockdown_controllers=(source-controller kustomize-controller helm-controller notification-controller)
flux_lockdown_images=(
  ghcr.io/fluxcd/source-controller:v1.10.1@sha256:611b6bca3eae95bcff471414d1fb14a4ab83d7f14e609cccef2bd389c121f37e
  ghcr.io/fluxcd/kustomize-controller:v1.10.1@sha256:37db29b25dae35b881a5e064acedb0cc954442da453a39eaa0eab7a90dbecc4e
  ghcr.io/fluxcd/helm-controller:v1.7.1@sha256:54526cc6ed1202d71d44c310676448065664f3255ad671d9a69ac26f904e617a
  ghcr.io/fluxcd/notification-controller:v1.10.0@sha256:9f58b9532ec9d3ccf539cd3ccdf283477a5dcb7777b922db5477d8bbd4d3767c
)
flux_lockdown_expected_args=(
  "kustomize-controller --no-cross-namespace-refs=true"
  "kustomize-controller --no-remote-bases=true"
  "kustomize-controller --default-service-account=default"
  "helm-controller --no-cross-namespace-refs=true"
)

# Usage: flux_lockdown_render <platform-dir> <flux-version>. Prints the patched install manifests.
flux_lockdown_render() {
  local platform_dir=$1
  local version=$2
  local work
  work=$(mktemp -d)
  flux install --version="$version" --components="$flux_lockdown_components" \
    --network-policy=false --export >"$work/gotk-components.yaml"
  cp -R "$platform_dir/components/flux-lockdown" "$work/flux-lockdown"
  cat >"$work/kustomization.yaml" <<YAML
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - gotk-components.yaml
components:
  - flux-lockdown
YAML
  local status=0
  kustomize build "$work" || status=$?
  rm -rf -- "$work"
  return "$status"
}

# Usage: flux_lockdown_wait. Waits for every installed controller to roll out.
flux_lockdown_wait() {
  local controller
  for controller in "${flux_lockdown_controllers[@]}"; do
    kubectl -n flux-system rollout status "deployment/$controller" --timeout=5m
  done
}

# Usage: flux_lockdown_missing. Prints one "<controller> <flag>" line per lockdown flag the live
# controllers lack, one "<controller> image <ref>" line per controller not running its pinned
# image, and "flux-system serviceAccountName" when the bootstrap Kustomization exists without
# spec.serviceAccountName=kustomize-controller.
flux_lockdown_missing() {
  local entry controller flag args
  for entry in "${flux_lockdown_expected_args[@]}"; do
    controller=${entry%% *}
    flag=${entry#* }
    args=$(kubectl -n flux-system get deployment "$controller" \
      -o jsonpath='{range .spec.template.spec.containers[0].args[*]}{@}{"\n"}{end}' 2>/dev/null) || args=
    grep -qxF -- "$flag" <<<"$args" || printf '%s %s\n' "$controller" "$flag"
  done
  local image live_image
  for image in "${flux_lockdown_images[@]}"; do
    controller=${image##*/}
    controller=${controller%%:*}
    live_image=$(kubectl -n flux-system get deployment "$controller" \
      -o jsonpath='{.spec.template.spec.containers[0].image}' 2>/dev/null) || live_image=
    [[ $live_image == "$image" ]] || printf '%s image %s\n' "$controller" "$image"
  done
  local sync_sa
  if sync_sa=$(kubectl -n flux-system get kustomizations.kustomize.toolkit.fluxcd.io flux-system \
    -o jsonpath='{.spec.serviceAccountName}' 2>/dev/null); then
    [[ $sync_sa == kustomize-controller ]] || printf 'flux-system serviceAccountName\n'
  fi
}

# Usage: flux_lockdown_verify <platform-dir>. Fails with the missing flags and the fix.
flux_lockdown_verify() {
  local platform_dir=$1
  local missing
  missing=$(flux_lockdown_missing)
  if [[ -n $missing ]]; then
    printf 'ERROR: Flux multi-tenancy lockdown is incomplete:\n%s\n' "$missing" >&2
    printf 'Add the patches and images in %s/components/flux-lockdown/kustomization.yaml to the GitOps\n' \
      "$platform_dir" >&2
    printf 'repository flux-system/kustomization.yaml and run up again.\n' >&2
    return 1
  fi
  printf 'Flux multi-tenancy lockdown is active.\n'
}
