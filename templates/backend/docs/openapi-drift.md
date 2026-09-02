# OpenAPI drift

`backend/openapi.json` is the contract. The Rust code is the only thing allowed
to author it, and every client reads it. The `api-drift` job in
`.github/workflows/ci.yml` enforces both halves.

## Why a committed file at all

The specification could be generated on demand, but then no reviewer ever sees
an API change. Committing it puts every added field, renamed property, and
removed endpoint into the diff, next to the code that caused it. The job's job
is to make sure that file cannot lie.

## What the job checks

1. **The specification matches the code.** It copies the committed file aside,
   runs `scripts/openapi.sh` to regenerate it in place, and `diff -u`s the two.
   The diff is deliberate: a failure prints the actual delta, so the fix is
   obvious without a local reproduction.
2. **Every consumer matches the specification.** It reruns
   `scripts/openapi-client.sh` and fails if any generated declaration changed.
   A consumer that has never been generated is reported and skipped, since it
   has nothing to drift from yet.

`backend/tests/openapi_drift.rs` runs the first check as a plain `cargo test`,
so the same failure appears locally before a push.

## Multiple consumers

One specification usually feeds several clients: a web app, a mobile app, an MCP
server, a partner SDK. They drift independently, and the one nobody regenerated
is the one that breaks in production.

Add each consumer to `openapi.consumers` in `baukit.toml`. The script reads that
list, and the workflow checks each committed output. A local
`sh scripts/openapi-client.sh` and CI therefore cover the same files.

```toml
[openapi]
schema = "backend/openapi.json"
consumers = ["generated/openapi.d.ts", "web/src/api/schema.d.ts"]
```

Commit every generated file the script writes. An untracked one passes a `git
diff` check without ever being compared.

## When the job fails

Regenerate and commit. Do not hand-edit `backend/openapi.json`, and do not
regenerate only the consumer that broke. If the change was not intended, the
failure is telling you the code changed the public API by accident, which is the
whole reason the gate exists.
