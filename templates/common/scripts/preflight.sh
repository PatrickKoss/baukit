#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

uses_git_dependency=false
if grep -Eq '^source[[:space:]]*=[[:space:]]*"git"' "$repository_root/baukit.toml"; then
  uses_git_dependency=true
fi

if [ "${BAUKIT_PREBUILT_IMAGES:-false}" = "true" ]; then
  echo "preflight: prebuilt-image mode skips build-only SSH checks"
elif [ "$uses_git_dependency" = "true" ]; then
  if [ -z "${SSH_AUTH_SOCK:-}" ]; then
    echo 'preflight: SSH_AUTH_SOCK is unset; start an agent with `eval "$(ssh-agent -s)"`, then load a key with `ssh-add ~/.ssh/<private-key>`.' >&2
    exit 1
  fi
  if [ ! -S "$SSH_AUTH_SOCK" ]; then
    echo 'preflight: SSH_AUTH_SOCK does not point to a socket; restart the agent, then load a key with `ssh-add ~/.ssh/<private-key>`.' >&2
    exit 1
  fi
  if ! command -v ssh-add >/dev/null 2>&1; then
    echo 'preflight: ssh-add is unavailable; install an OpenSSH client, start an agent, and load a key.' >&2
    exit 1
  fi
  set +e
  ssh-add -l >/dev/null 2>&1
  ssh_add_status=$?
  set -e
  case "$ssh_add_status" in
    0) ;;
    1)
      echo 'preflight: the SSH agent has no loaded identities; add one with `ssh-add ~/.ssh/<private-key>`.' >&2
      exit 1
      ;;
    2)
      echo 'preflight: the SSH agent is unusable; restart it, then add a key with `ssh-add ~/.ssh/<private-key>`.' >&2
      exit 1
      ;;
    *)
      echo 'preflight: could not query the SSH agent; restart it, then add a key.' >&2
      exit 1
      ;;
  esac
fi

has_playwright=false
if [ -f "$repository_root/web/package.json" ] && grep -Eq '"(@playwright/test|playwright)"' "$repository_root/web/package.json"; then
  has_playwright=true
fi

if [ "$has_playwright" = "true" ]; then
  PLAYWRIGHT_BROWSERS_PATH="$repository_root/web/node_modules/.cache/playwright-browsers"
  export PLAYWRIGHT_BROWSERS_PATH
  if ! command -v corepack >/dev/null 2>&1; then
    echo 'preflight: Playwright requires current Node.js LTS with corepack.' >&2
    exit 1
  fi
  if [ ! -d "$repository_root/web/node_modules/@playwright/test" ]; then
    (
      cd "$repository_root/web"
      corepack pnpm@11.18.0 install --frozen-lockfile
    )
  fi
  if [ ! -d "$repository_root/web/node_modules/@playwright/test" ]; then
    echo 'preflight: the installed web dependencies do not provide Playwright.' >&2
    exit 1
  fi

  check_playwright_browsers() (
    cd "$repository_root/web"
    corepack pnpm@11.18.0 exec node -e '
      const fs = require("node:fs");
      const path = require("node:path");
      const { chromium, webkit } = require("@playwright/test");
      const cache = fs.realpathSync(path.resolve(process.env.PLAYWRIGHT_BROWSERS_PATH));
      for (const browser of [chromium, webkit]) {
        const executable = fs.realpathSync(browser.executablePath());
        const relative = path.relative(cache, executable);
        if (relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
          throw new Error("Playwright executable resolved outside the repository cache");
        }
        fs.accessSync(executable, fs.constants.X_OK);
      }
    ' >/dev/null 2>&1
  )

  if ! check_playwright_browsers; then
    (
      cd "$repository_root/web"
      corepack pnpm@11.18.0 exec playwright install chromium webkit
    )
    if ! check_playwright_browsers; then
      echo 'preflight: Playwright browser executables are missing from the repository cache.' >&2
      exit 1
    fi
  fi
fi

echo "preflight: environment is ready"

if [ "${1:-}" = "--" ]; then
  shift
fi
if [ "$#" -gt 0 ]; then
  cd "$repository_root"
  exec "$@"
fi
