# shellcheck shell=sh
# Sourced by deploy validators. Usage: baukit_use_tools <repo-root> <tool>...
# Puts $BAUKIT_TOOLS_DIR/bin (default <repo-root>/.tools/bin) first on PATH. Exits 2 when a
# tool is missing or not at the version pinned in deploy/tools.toml.

baukit_use_tools() {
    baukit_tools_root=$1
    shift
    baukit_tools_dir=${BAUKIT_TOOLS_DIR:-$baukit_tools_root/.tools}
    baukit_tools_bin=$baukit_tools_dir/bin
    baukit_tools_stale=$(python3 - "$baukit_tools_root" "$baukit_tools_dir" "$@" <<'PY'
import pathlib
import sys
import tomllib

root = pathlib.Path(sys.argv[1])
tools_dir = pathlib.Path(sys.argv[2])
tools = tomllib.loads((root / "deploy/tools.toml").read_text(encoding="utf-8"))["tools"]
for name in sys.argv[3:]:
    marker = tools_dir / "markers" / name
    binary = tools_dir / "bin" / name
    pinned = tools[name]["version"] if name in tools else None
    installed = marker.read_text(encoding="utf-8").split(" ")[0] if marker.is_file() else None
    if pinned is None or installed != pinned or not binary.is_file():
        print(name)
PY
    ) || exit 2
    if [ -n "$baukit_tools_stale" ]; then
        baukit_tools_list=$(printf '%s' "$baukit_tools_stale" | tr '\n' ',')
        printf 'ERROR: pinned tools missing or outdated in %s: %s\n' "$baukit_tools_bin" "$baukit_tools_list" >&2
        printf 'Run: scripts/install-deploy-tools.sh --only %s\n' "$baukit_tools_list" >&2
        exit 2
    fi
    PATH=$baukit_tools_bin:$PATH
    export PATH
}

# Prints the version deploy/tools.toml pins for a tool. Usage: baukit_tool_version <repo-root> <tool>
baukit_tool_version() {
    python3 - "$1" "$2" <<'PY'
import pathlib
import sys
import tomllib

manifest = pathlib.Path(sys.argv[1], "deploy/tools.toml").read_text(encoding="utf-8")
tool = tomllib.loads(manifest)["tools"].get(sys.argv[2])
if tool is None:
    sys.exit(f"ERROR: deploy/tools.toml pins no tool named {sys.argv[2]}")
print(tool["version"])
PY
}
