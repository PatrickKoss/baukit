# Evidence 24: provider credential probes

## Source product files

- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-bin/src/adapters/integrations/github.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-bin/src/adapters/integrations/toggl.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-ports/src/integrations.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-services/src/integrations/mod.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-bin/src/adapters/integrations/fake.rs`

## Observed repeated glue

GitHub and Toggl each call an account endpoint, classify revoked, missing scope,
rate limited, timeout, unavailable, and invalid data, then extract an external
account ID. GitHub also reads a scope header after a successful response. Both
adapters read successful bodies without an explicit byte limit and discard
`Retry-After` on rate limits.

## Baukit owner

`baukit-integrations` owns `CredentialProbe`, opaque account identity, the
closed failure set, connection-health mapping, response bounds, and retry-delay
transport. `baukit-test` owns the scripted HTTP server and conformance runner.

## Public types and errors

`CredentialProbe`, `CredentialProbeFuture`, `CredentialProbeSuccess`,
`ExternalAccountId`, `InvalidExternalAccountId`, and `CredentialProbeError`.
The conformance API exposes `CredentialProbeConformanceCases`,
`ScriptedCredentialProbeHttp`, and check and assertion functions.

## Product-owned inputs

Products keep endpoints, authorization headers, required scopes, provider
statuses, response parsing, account models, public API codes, and recovery copy.

## Cases

- Concurrency: probe implementations are `Send + Sync`; connection persistence
  and deduplication remain product work.
- Failure: revoked, missing scope, rate limited with or without a retry hint,
  timeout, unavailable, invalid data, and oversized data.
- Privacy: credentials and provider bodies are absent from public error types;
  account-ID debug output is redacted; the fake retains only a call count.
- Cleanup: dropping the scripted server aborts its task; a pending response is
  bounded by the adapter timeout and the conformance deadline.

## Supported runtimes

Rust 1.95 or newer with any async HTTP client. The production crate has no HTTP
runtime or client dependency. The conformance server uses Tokio on loopback.

## Product adoption change

After Tiefgang pins the release, replace `TokenProviderConnector`,
`TokenProviderError`, `ProviderAccount`, and `FakeTokenConnector`. Implement
`CredentialProbe` for `GithubTokenAdapter` and `TogglTokenAdapter`, run both
through `check_credential_probe_conformance`, and keep their endpoint, auth,
scope, and JSON code local.
