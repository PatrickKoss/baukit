## Shared agent rules (baukit)

`baukit check agent-docs` keeps this block identical to `lint/agent-rules.md` in baukit.
Edit it there, not here.

### Done means the gate passed

- Run the repo gate before you call a task done. Rust tests run with `-- --include-ignored`, so the
  Docker-gated tests run too. A red gate is never "unrelated" until you have read the failure.
- `baukit check` must pass. Every rule it enforces is listed below.

### Code

- Production code does not panic. `unwrap`, `expect`, `panic!`, indexing and lossy `as` casts are lint
  errors outside tests. Propagate the error or encode the invariant in the type (`let ... else`, `get`,
  `try_from`). `unwrap_or_default` or `.ok()` on a failure that is a bug is not a fix.
- Suppressions carry a reason that names the invariant. Use `#[expect(lint, reason = "...")]`, not
  `#[allow]`. Comment forms (`eslint-disable`, `ast-grep-ignore`) need `-- reason`. The counts in
  `baukit-check.toml` only go down.
- No file grows past 1000 lines (Rust) or 500 lines (TS). Allowlisted files may not grow at all: split them.
- Domain logic stays free of frameworks. I/O sits behind a port with an adapter, and services orchestrate.
- TypeScript parses untrusted data with guards (`@baukit/type-guards`), never `as unknown as T`.

### Security

- SQL on tenant data puts the owner in the WHERE: `WHERE owner_id = $1 AND id = $2`. Loading by id and
  comparing the owner in Rust is a finding. Every route that takes an id gets a test where a second
  principal is denied.
- Every route sits inside the rate limiter, including unauthenticated callbacks, webhooks and
  `/platform/*` routes merged late.
- Outbound HTTP goes through `baukit-egress`. Do not build a raw `reqwest::Client`.
- Types holding secrets never derive `Debug`. Use `secrecy` or a redacting impl. Never log a token.
- Browser code never writes a token to `localStorage`, `sessionStorage`, IndexedDB or caches.
- New dependencies must pass `cargo deny` and `osv-scanner`. Actions are pinned by commit SHA.
- `docs/threat-model.md` lists every trust boundary. When you add a boundary (route, webhook, integration,
  queue, file upload), update it with the `baukit-threat-model` skill and test the controls with
  `baukit-security-testing`.
