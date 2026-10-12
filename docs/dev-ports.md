# Dev ports

Declare local dev servers in the product's `baukit.toml`:

```toml
[ports]
web = { host = 5673 }
web_preview = { host = 4773 }
metro = { host = 8093 }
keycloak = { host = 8091, container = 8080 }
```

Each dev-server key is optional and accepts only `host`. `container` and
`service` are errors for these keys. Existing service declarations keep their
host, container, and optional service fields.

`web` sets Vite's dev port, bundled Keycloak web origins and redirect URIs,
the backend's authorized web origin, the PKCE helper's redirect default, and
the Keycloak stack test's `E2E_WEB_PORT` default. `web_preview` sets Vite's
preview port and the hermetic Playwright server's `E2E_WEB_PORT` default.
`metro` adds `--port` to mobile scripts that start Expo or run a native app.

Absent declarations retain Vite dev 5173, preview 4173, and Expo's unpinned
Metro port. `port_offset` shifts PostgreSQL, API, operations, Keycloak, Redis,
and fake-provider host ports. It never shifts these dev-server ports. Doctor
skips undeclared dev servers. All declared hosts must differ from each other
and the effective service hosts. In particular, declaring Metro on 8081
collides with default Keycloak, so move Keycloak or choose another Metro port.

## Re-rendering

The CLI has no `upgrade` command. `baukit new --force` and
`baukit new --into-existing` read the destination manifest's ports. They
preserve existing files and the manifest, and write differences to
`baukit-conflicts.txt`. A conflicting render exits with an error.

To review a port change, copy the complete updated `baukit.toml` into an empty
directory, then run `baukit new` with the same name and capability flags.
Repeat `--baukit-path` if the product uses a local checkout:

```sh
mkdir -p .port-render
cp baukit.toml .port-render/
baukit new my-product --backend --web --mobile --auth oidc \
  --into-existing --dir .port-render --skip-lockfiles
```

Copy the affected rendered files into the product after reviewing them. Keep
product changes in those files. Remove `.port-render` when the review is done.
Run `baukit doctor` from the product root to check the result.

## Doctor checks

Doctor checks only declared dev ports. It skips absent files, so a product can
relocate or remove a template file. Its separate expected-file checks still
apply unless the product configures a doctor waiver.

- `web`: `web/vite.config.ts` must set `server.port`. The stack Playwright
  config must have the same `E2E_WEB_PORT` default. For bundled Keycloak,
  localhost dev origins in `keycloak/realm.json`, `keycloak/reconcile.json`,
  backend `authorized_parties`, and `scripts/pkce-login.py` must use this port.
  Production origins, mobile callback schemes, and the MCP client's separate
  OAuth callback do not affect this check.
- `web_preview`: `web/e2e/playwright.config.ts` must use the declared
  `E2E_WEB_PORT` default.
- `metro`: every `expo start` or `expo run:*` command in `mobile/package.json`
  must pass `--port`. In `scripts/`, `mobile/scripts/`, and the root Makefile,
  doctor checks literal ports in `adb ... reverse` lines only when the line
  contains `metro`, in any letter case. This includes variables such as
  `$METRO_PORT` and `${metro_port}`. API and Keycloak reversals without a Metro
  name are skipped. `RCT_METRO_PORT=N` settings must also agree. The scan skips
  `node_modules`, `.git`, `target`, and `__pycache__` directories. Doctor reports
  the file and any mismatched port.

Runtime environment overrides remain product-owned. Doctor compares the
committed defaults with the manifest, not the environment of a running server.
