#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-deploy-tools.sh [--only name,...]

Installs the validator tools pinned in deploy/tools.toml into .tools/bin and verifies each sha256.
A tool whose installed version and checksum already match is skipped.

Environment:
  BAUKIT_TOOLS_MANIFEST  manifest path (default: deploy/tools.toml)
  BAUKIT_TOOLS_DIR       install root (default: .tools)
  BAUKIT_TOOLS_PLATFORM  platform id such as linux-amd64 (default: detected)
EOF
}

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
manifest=${BAUKIT_TOOLS_MANIFEST:-$repo_root/deploy/tools.toml}
tools_dir=${BAUKIT_TOOLS_DIR:-$repo_root/.tools}
only=""

while (($# > 0)); do
  case "$1" in
    --only)
      [[ $# -ge 2 ]] || { usage >&2; exit 2; }
      only=$2
      shift 2
      ;;
    --only=*)
      only=${1#--only=}
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      printf 'ERROR: unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

detect_platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    *) os=$(uname -s) ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) arch=$(uname -m) ;;
  esac
  printf '%s-%s\n' "$os" "$arch"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

platform=${BAUKIT_TOOLS_PLATFORM:-$(detect_platform)}
work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT

plan=$(python3 - "$manifest" "$platform" "$only" "$work_dir" <<'PY'
import hashlib
import pathlib
import sys
import tomllib

manifest_path, platform, only, work_dir = sys.argv[1:5]
manifest = tomllib.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
tools = manifest.get("tools", {})
if platform not in manifest.get("platforms", []):
    sys.exit(f"ERROR: unsupported platform {platform}; {manifest_path} supports {', '.join(manifest.get('platforms', []))}")
selected = [name.strip() for name in only.split(",") if name.strip()] if only else list(tools)
unknown = [name for name in selected if name not in tools]
if unknown:
    sys.exit(f"ERROR: unknown tool(s): {', '.join(unknown)}; known: {', '.join(tools)}")

os_part, arch_part = platform.split("-", 1)
for name in selected:
    tool = tools[name]
    version = tool["version"]
    if "requirements" in tool:
        text = "\n".join(tool["requirements"]) + "\n"
        pathlib.Path(work_dir, f"{name}.requirements").write_text(text, encoding="utf-8")
        digest = hashlib.sha256(text.encode()).hexdigest()
        print("\t".join([name, version, "pip", digest, "-"]))
        continue
    digest = tool.get("sha256", {}).get(platform)
    if not digest:
        sys.exit(f"ERROR: {name} has no sha256 for platform {platform}")
    fields = {
        "version": version,
        "os": tool.get("os_names", {}).get(os_part, os_part),
        "arch": tool.get("arch_names", {}).get(arch_part, arch_part),
    }
    url = tool["url"].format(**fields)
    member = tool["member"].format(**fields) if "member" in tool else "-"
    print("\t".join([name, version, url, digest, member]))
PY
)

bin_dir=$tools_dir/bin
marker_dir=$tools_dir/markers
mkdir -p "$bin_dir" "$marker_dir"

install_pip() {
  local name=$1 lib_dir=$tools_dir/lib/$1
  rm -rf -- "$lib_dir"
  python3 -m pip install --quiet --disable-pip-version-check --no-deps --require-hashes \
    --only-binary :all: --target "$lib_dir" --requirement "$work_dir/$name.requirements"
  printf '#!/bin/sh\nPYTHONPATH="%s" exec python3 -m %s "$@"\n' "$lib_dir" "$name" >"$bin_dir/$name"
  chmod 0755 "$bin_dir/$name"
}

install_download() {
  local name=$1 url=$2 expected=$3 member=$4
  local download=$work_dir/$name.download actual
  curl --fail --silent --show-error --location --retry 3 --output "$download" "$url"
  actual=$(sha256_of "$download")
  if [[ "$actual" != "$expected" ]]; then
    printf 'ERROR: checksum mismatch for %s from %s\n  expected %s\n  actual   %s\n' \
      "$name" "$url" "$expected" "$actual" >&2
    exit 1
  fi
  if [[ "$member" == "-" ]]; then
    install -m 0755 "$download" "$bin_dir/$name"
    return
  fi
  mkdir -p "$work_dir/$name.extract"
  tar -xzf "$download" -C "$work_dir/$name.extract" "$member"
  install -m 0755 "$work_dir/$name.extract/$member" "$bin_dir/$name"
}

while IFS=$'\t' read -r name version source digest member; do
  marker="$version $digest"
  if [[ -x "$bin_dir/$name" && -f "$marker_dir/$name" && "$(cat "$marker_dir/$name")" == "$marker" ]]; then
    printf 'ok %s %s (already installed)\n' "$name" "$version"
    continue
  fi
  rm -f -- "$marker_dir/$name"
  if [[ "$source" == "pip" ]]; then
    install_pip "$name"
  else
    install_download "$name" "$source" "$digest" "$member"
  fi
  printf '%s\n' "$marker" >"$marker_dir/$name"
  printf 'installed %s %s\n' "$name" "$version"
done <<<"$plan"
