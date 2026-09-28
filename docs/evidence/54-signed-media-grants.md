# Signed media grants evidence

Plan item 18, "Add signed media grants". Shared vectors, a Rust signer and verifier behind the
`baukit-core` feature `media-grants`, and an njs verifier under `deploy/media-grants` are
implemented. The chart stays product-owned. Product adoption is deferred.

## Source revisions

- Baukit baseline `523a233`.
- Eigenruhe `f74cebb`. The working tree had unrelated uncommitted animation files; none of the
  files read here were modified.
- Hebkit `841bf5d`, clean.

## Step 1: the two implementations compared

Both products sign the same bytes, `"{path}\n{expires}\nplayback\n{key_id}"` in UTF-8, with
HMAC-SHA256, and encode the tag as unpadded URL-safe base64. Both keep a current and an optional
previous key. The table lists every difference found at byte level or in accept and reject
behavior.

| Point | Eigenruhe | Hebkit | Baukit |
|---|---|---|---|
| HMAC key bytes | base64url-decoded secret | raw UTF-8 bytes of the configured string | base64url-decoded secret |
| Secret length | decoded at least 32 bytes | UTF-8 at least 32 bytes | decoded at least 32 bytes |
| Secret encoding checks | canonical, unpadded, re-encode must match | none | canonical, unpadded, re-encode must match |
| Key ID grammar (Rust) | `[A-Za-z0-9][A-Za-z0-9_-]*`, at most 64 | starts alphanumeric, at most 64 | `[A-Za-z0-9][A-Za-z0-9_-]*`, at most 64 |
| Key ID grammar (njs) | same as Rust | `[A-Za-z0-9_-]{1,64}`, so a leading `_` or `-` passes | same as Rust |
| Key ID parameter name | `key_id` | `key_id` | `keyId` |
| Parameter order | exactly `expires, key_id, mode, signature` | any order, each once | exactly `expires, keyId, mode, signature` |
| Query length cap | 158 bytes | none | 157 bytes (one byte shorter name) |
| `expires` digits | 1 to 10, no leading zero | 1 to 12, no leading zero | 1 to 10, no leading zero |
| Default lifetime | 300 s | 900 s | caller's choice |
| Lifetime bounds | 1 to 3600 s | 60 to 3600 s | signer at most 3600 s |
| Clock skew | none; `expires - now > 3600` fails | none; same | verifier accepts up to 3660 s ahead |
| Expired at | `expires <= now` | `expires <= now` | `expires <= now` |
| Path grammar | product allowlist of animation and attribution paths | product allowlist of animation and attribution paths | generic segment grammar, allowlist moves to nginx `location` |
| Raw vs normalized path | Rust takes both and requires equality; njs compares `$request_uri` path to `$uri` | njs compares `$request_uri` path to `$uri` | Rust takes the raw path only; njs compares `$request_uri` path to `$uri` |
| Request line | njs rejects absolute-form targets | no check | no check |
| Signature decode order | after key lookup, so bad encoding with an unknown key is `unknown_key` | regex before key lookup | during query parsing, so bad encoding is always `invalid_query` |
| Rust compare | `hmac` `verify_slice`, constant time | none (signs only) | `ring::hmac::verify`, constant time |
| njs compare | `crypto.subtle.verify` | XOR over base64url characters | `crypto.subtle.sign`, then XOR over all 32 bytes |
| njs key source | `js_set` variables filled from `process.env` | `process.env` | `process.env` |
| njs engine | QuickJS | native njs | both, tested |
| Cache-Control | `private, max-age=<left>, must-revalidate`; error statuses drop it | same value, `no-store` on error, recomputes the HMAC twice per response | same as Eigenruhe |
| Other headers | nginx `add_header` for CORS, nosniff, Referrer-Policy | the header filter sets CORS, nosniff, Referrer-Policy, Vary | left to product nginx config |

Sources:

- Eigenruhe signer `backend/crates/eigenruhe-bin/src/animation_signing.rs:6-11` (constants),
  `:69` (`SigningKey::from_base64url`), `:162-181` (`sign_playback`), `:183-212`
  (`verify_playback_grant`), `:249` (query cap). Callers in `animation_media.rs:55`, `lib.rs:171-181`
  (TTL validation), and `bin/api.rs:29`.
- Eigenruhe verifier `media/njs/animation-signing.js`, its `animation-signing.test.mjs`, the
  `environment.js` key loader, `media/nginx.conf`, and the `Makefile:163` target
  `node --test media/njs/*.test.mjs`.
- Eigenruhe fixtures `content/fixtures/animation-signing.json`, 20 cases with snake_case keys.
- Eigenruhe chart `deploy/animation-media/`.
- Hebkit signer `backend/crates/hebkit-bin/src/exercise_media.rs:54-100` and TTL config at
  `config.rs:432`.
- Hebkit verifier `infra/exercise-media/authorize.js:1-52`, `nginx.conf`, `test.sh` (its
  previous key is the 32-byte UTF-8 string `éééééééééééééééé`), and the manifest
  `deploy/exercise-media.yaml`.

The HMAC key difference matters most. Hebkit's `MEDIA_SIGNING_KEY` value is HMAC key material
as typed, while Eigenruhe and Baukit decode it first. The same configured string therefore gives
different signatures, and Hebkit's development key `hebkit-development-media-signing-key` is not
valid base64url of 32 bytes. Hebkit must mint new keys when it adopts.

## Step 2: shared vectors

`fixtures/media-grants/vectors-v1.json` (SHA-256 `5583fa66415a254d62507ca70c03af62a7408df20aaef227ec0cff3e7550949f`)
holds:

- `protocol`, the constants both verifiers export and check against.
- `errorCodes` for keys and grants.
- Four keys. `current` and `previous` reuse Eigenruhe's secrets under the new IDs
  `current_2026_09` and `previous_2026_08`. `next` is bytes `0x40..0x5f`. `utf8` decodes to
  non-ASCII UTF-8 text, so a verifier that keys HMAC with the text instead of the bytes fails.
- 10 `keyCases`: short, padded, non-canonical, standard-base64, raw-text, and empty secrets, plus
  an empty key ID, one containing `&`, one of 65 bytes, and one of 64 bytes that loads.
- 3 `ringCases`: a duplicate key ID, current only, and next with current as previous.
- 9 `signCases`: four signed grants (current, previous, the UTF-8 secret, and the full 3600 s)
  and the signer's refusals of 3601 s, expired, traversal, an encoded path, and an 11-digit
  expiry.
- 50 `verifyCases`, all at `now = 2000000000`, covering:
  - expiry: one second left, exactly 3600 s, `expires == now`, past, and a verifier clock ahead
    of the signer;
  - clock skew: 3660 s ahead passes, 3661 s fails with `expiry_too_far`;
  - rotation with two live keys: the previous key still verifies, a retired previous key is
    `unknown_key`, and after promoting `next` both `next` and the old current key verify;
  - tampering: path, expiry, and a signature made under another key ID;
  - methods: `POST` and lowercase `get`;
  - path normalization: `%2e%2e`, `%61`, `..`, `.`, a hidden segment, `//`, a trailing slash, `/`
    alone, a relative path, a backslash, a space, non-ASCII, and 513 bytes;
  - the encoded traversal `/media/<uuid>%2Fv1%2F..%2F..%2Fsecret.mp4`;
  - query grammar: empty, `key_id`, reordered, duplicate, extra, `mode=download`, padded,
    non-canonical tail, 42 characters, standard base64, a leading zero, 11 digits, a 65-byte key
    ID, and a leading underscore.

The expected values came from a 307-line Python standard-library script,
`media_grants_gen.py`, run once from the session scratchpad and not committed. It never imports
or runs Baukit code. Before it wrote anything it asserted that it reproduces three product
signatures:

| Source | Key | Expected signature |
|---|---|---|
| Eigenruhe fixture, current-key video | `current_2026_09` | `BO9XiWyLvQsOrnjQFtECdtRS9vAUoNrQUWwGtX9MvtA` |
| Eigenruhe fixture, unicode attribution | `unicode_2026_09` | `sojONUY80e1NuvxpMqgTGP9D2XAiGwxbhBFQXcuaw4E` |
| Hebkit `exercise_media.rs` test, raw UTF-8 key | `local-v1` | `WeA1jOoQwIcqXo3Eli16DC_TMKTJmGxF83NFs4wSklo` |

All three matched, which confirms the signing input and the key-byte difference above.

## Baukit owner

The Rust code lives in `baukit-core` behind the new feature `media-grants`, not in `baukit-http`
as the plan table proposed. The module needs `ring`, `base64`, and `zeroize`. `baukit-core`
already carries `ring` and `base64` as optional dependencies for `pagination`, and `zeroize` is
already a workspace dependency. `baukit-http` pulls in Axum, Tower, Tokio, and OpenTelemetry, and
a worker or domain crate that only signs URLs should not compile an HTTP framework. A new crate
would add publishing and version-coherence work for about 500 lines. The feature adds no
dependency to the default build. `rust/Cargo.lock` gains two edges for `baukit-core`:
`zeroize` and the self dev-dependency that turns the feature on for its own tests.

The njs verifier lives in `deploy/media-grants/njs/`, next to the other deployable artifacts.

## Public types and functions

`baukit_core::media_grant`, behind `media-grants`:

- Constants: `MEDIA_GRANT_MODE`, `MAX_GRANT_LIFETIME_SECONDS` (3600), `MAX_CLOCK_SKEW_SECONDS`
  (60), `MIN_SECRET_BYTES` (32), `MAX_KEY_ID_BYTES` (64), `MAX_PATH_BYTES` (512).
- `MediaGrantKey::from_base64url(key_id, secret)`, `key_id()`, and `sign(path, expires, now)`.
- `MediaGrantKeyRing::new(current, previous)`, `current()`, `previous()`, `sign`, and
  `verify(MediaGrantRequest)`.
- `MediaGrant` with `expires()`, `key_id()`, and `query()`.
- `MediaGrantRequest { method, path, query, now }`.
- `VerifiedMediaGrant` with `expires()` and `key_id()`.
- `MediaGrantKeyError` and `MediaGrantError`, each with `code()`.
- `signing_input(path, expires, key_id)` and `valid_media_path(path)`.

`deploy/media-grants/njs/media-grant.js` default export: `authorize` (`js_access`),
`responseHeaders` (`js_header_filter`), `verifyMediaGrant`, `loadMediaGrantKey`,
`loadMediaGrantKeyRing`, `loadMediaGrantKeyRingFromEnv`, `signingInput`, `validMediaPath`, and
`protocol`.

## Contract as implemented

- Query: `expires=<1-10 digits, no leading zero>&keyId=<id>&mode=playback&signature=<43 chars>`,
  exact order, nothing else, at most 157 bytes.
- Signature: unpadded base64url of HMAC-SHA256 over `"{path}\n{expires}\nplayback\n{keyId}"`,
  keyed with the decoded secret. It must decode canonically to 32 bytes.
- Path: leading `/`, at most 512 bytes, segments of `[A-Za-z0-9._-]` that are non-empty and do
  not start with `.`. Such a path contains no `%`, no dot segment, and no empty segment, so
  nginx's `$uri` equals it byte for byte and the Rust verifier needs no normalized path.
- Expiry: the signer refuses `expires <= now` and more than 3600 s ahead. The verifier refuses
  `expires <= now` and more than 3660 s ahead.
- Keys: `[A-Za-z0-9][A-Za-z0-9_-]*` IDs of at most 64 bytes, canonical unpadded base64url
  secrets of at least 32 decoded bytes, and distinct IDs in a ring.
- Check order: method, path, query, expiry, key, signature. Error codes are `invalid_method`,
  `invalid_path`, `invalid_query`, `expired`, `expiry_too_far`, `unknown_key`, and
  `invalid_signature`. Key errors are `invalid_key_id`, `invalid_secret_encoding`,
  `secret_too_short`, and `duplicate_key_id`.
- njs adapter: `$request_uri` must contain `?` after a non-empty path, and that path must equal
  `$uri`. On success it sets `$media_grant_cache_control` to
  `private, max-age=<expires - now>, must-revalidate`. The header filter writes it for statuses
  below 400 and deletes `Cache-Control` otherwise.

## Decisions

- Clock skew. Both products reject anything more than 3600 s ahead, so a 3600 s grant fails at
  an edge whose clock lags the API by one second. The verifier now allows 60 s; the signer
  still caps at 3600 s. Expiry has no grace.
- `key_id` becomes `keyId`, following the camelCase rule for new query parameters. The name is
  not part of the signed bytes, so signatures do not change.
- The product path allowlists leave the verifier. A generic grammar rejects every encoding and
  normalization trick in the vectors, and each product keeps its allowlist in nginx `location`
  regexes.
- Eigenruhe's absolute-form request-line check is dropped. nginx fills `$request_uri` with the
  path and query only, so an absolute-form request is still bound to the signed path. A live run
  confirmed that such a request gets the same answer as the origin-form one.
- Keys reach njs through `process.env` and `env` directives, not `js_set` variables, so no
  `log_format` can reference them. A broken key configuration refuses every request and logs
  only the error code.
- The njs compare signs and XORs all 32 bytes. `crypto.subtle.verify` would also work, but the
  explicit loop behaves the same in both engines and in Node.
- The chart stays product-owned. Eigenruhe's `deploy/animation-media` chart carries release-ID
  and index-hash readiness on a private port, a dedicated Traefik controller, product labels, a
  network policy, a PDB, and its own Python tests. Hebkit uses a plain Deployment, Service, and
  Ingress. The shared part is the njs file plus about 15 lines of nginx config, which the README
  documents. A generic chart would either copy one product's readiness model or be a thin
  wrapper around `nginx:alpine`. No chart code was added, so `helm lint` and `helm template` did
  not apply.

## Failure behavior

- The signer returns `MediaGrantError` for a bad path, an 11-digit expiry, an expired grant, or
  a lifetime over 3600 s. Key loading returns `MediaGrantKeyError`.
- The verifier returns the first failing check. The njs `authorize` handler answers 403 for
  every failure, including a thrown exception.
- Error `Display` strings are fixed and never include the path, query, key ID, or secret.

## Privacy boundary

- `MediaGrantKey` holds only a `ring::hmac::Key`. The decoded secret sits in a
  `Zeroizing<Vec<u8>>` that is wiped when loading returns. ring keeps the derived HMAC state
  without zeroizing it.
- `Debug` for `MediaGrantKey` shows the key ID and `<redacted>`. `MediaGrant` redacts the
  signature, and `MediaGrantRequest` redacts the whole query. A unit test checks that no `Debug`
  or error output contains the secret or the signature.
- In njs the secret lives in a closure, so `JSON.stringify(key)` shows only `keyId`. A Node test
  checks this.
- A grant query is a bearer credential until it expires. The README tells operators to log
  `$uri`, never `$request` or `$request_uri`, and to keep ingress logs and tracing from recording
  the query.

## Supported runtimes

- Rust: `baukit-core` with `media-grants`, MSRV 1.95.
- njs 1.0.1 in `nginx:1.31-alpine@sha256:df221db836e1754089190208cee7eeda94f233197056426eda74a43ab1abeac2`
  (nginx 1.31.6), both the native engine and QuickJS.
- Node 24 for tests.

The native engine rejected destructuring (`Token "a" not supported`), `for...of`
(`Token "of" not supported`), and named exports (`Non-default export is not supported`) during
probing, so the artifact exports one default object and uses index loops. The Node test needs no
runtime shim, since Node has `Buffer`, `crypto.subtle`, and `process.env`. It fakes the nginx
request object, mocks `Date` for the handler tests, and relies on `njs/package.json` with
`"type": "module"` so Node loads the `.js` files as ES modules without a warning.

## Verification of the njs artifact

- `make media-grants-test` runs 16 Node tests: the shared vectors in five groups, key
  serialization, previous-key configuration, and nine handler tests with a fake `r`.
- `make media-grants-njs-test` runs `run-njs-vectors.js` in the pinned image under both
  engines. Each prints `media grant vectors: 72 cases passed`. With one expectation changed in a
  scratch copy of the vectors, the runner printed the three mismatches and exited 1.
- A live smoke run started the pinned nginx image twice, once per engine, with the module and a
  config like the README's. Fresh grants from the current and the previous key returned 200 with
  `Cache-Control: private, max-age=300, must-revalidate`, and so did `HEAD`. A retired key,
  `POST`, a missing query, a tampered signature, a grant 3700 s ahead, an expired grant,
  `/media/clips/%63lip.mp4`, and the encoded traversal `/media/clips/..%2Fclips%2Fclip.mp4`
  returned 403. The access log showed only method, `$uri`, and status.
- The CI workflow gains a `media-grants` job that runs both targets. `make ci` runs them too.

## Breaks

For both products, on adoption:

- The query parameter `key_id` becomes `keyId`, so grants issued before the switch stop
  verifying. Grants live at most an hour and no product is live.
- Order and length rules tighten for Hebkit: exact parameter order, 10-digit expiry, key IDs
  starting alphanumeric.
- Hebkit's key material changes meaning from raw text to base64url, so its keys must be
  regenerated.
- The njs environment variables become `MEDIA_GRANT_KEY_ID`, `MEDIA_GRANT_SIGNING_KEY`,
  `MEDIA_GRANT_PREVIOUS_KEY_ID`, and `MEDIA_GRANT_PREVIOUS_SIGNING_KEY`.

No existing Baukit API changes.

## Product code to remove on adoption

- Eigenruhe: `backend/crates/eigenruhe-bin/src/animation_signing.rs` and its tests,
  `media/njs/animation-signing.js`, `media/njs/animation-signing.test.mjs`, the key getters in
  `media/njs/environment.js`, the four `js_set $animation_*_key*` lines in `media/nginx.conf`, and
  `content/fixtures/animation-signing.json`.
- Hebkit: `HmacPlaybackSigner` in `backend/crates/hebkit-bin/src/exercise_media.rs:54-100` and its
  test, the `validate`, `allowed`, and HMAC parts of `infra/exercise-media/authorize.js`, and the
  grant cases that `infra/exercise-media/test.sh` runs through `test_delivery.py`.

## Product adoption follow-ups (deferred)

- Eigenruhe: switch `HmacPlaybackSigner` to `MediaGrantKeyRing::sign`, map `MediaGrantError`
  codes, copy `deploy/media-grants/njs/media-grant.js` into the media image, rename the env
  variables, move the path allowlist into `location` regexes, keep `max_ranges`, CORS, and the
  release-readiness module, and delete the product fixtures.
- Hebkit: generate base64url keys, switch the signer to `MediaGrantKeyRing`, replace
  `authorize.js` with the Baukit module plus a small header filter for CORS and Referrer-Policy,
  replace `if ($media_allowed = 0)` with `js_access`, and raise the TTL floor question below.

## Open decisions

- Whether products should sign from a single `MediaGrantKeyRing` built at startup or rebuild on
  key reload. The type supports either; no product reloads keys today.
- Hebkit's TTL floor of 60 s and Eigenruhe's default of 300 s stay in product config. Baukit
  enforces only the 3600 s ceiling.
- Whether Baukit should ship the nginx config as a template file. The README snippet is enough
  while both products own their images.

## Product defects found

- Hebkit keys HMAC with the raw configured string. Any value of at least 32 bytes passes,
  including low-entropy text, and nothing checks that it is random key material.
- Hebkit's njs verifier accepts key IDs with a leading `_` or `-` that its Rust signer can never
  issue, and 11- or 12-digit expiries that the lifetime check then rejects anyway.
- Hebkit's header filter calls `validate(r)` twice for every response, so each served file costs
  three HMAC computations instead of one.
- Neither product allows clock skew. A grant signed at the full 3600 s lifetime fails at an edge
  whose clock is one second behind the API.

## Follow-up (2026-09-28)

This closes the key reload decision. Products build one `MediaGrantKeyRing` at startup and share
it. To rotate, they restart with the new key as current and the old one as previous, then restart
again without the previous key once the longest grant lifetime has passed. There is no reload API:
no product reloads keys today, and a restart already reloads the edge verifier's environment.

The `baukit-core` README section and the `MediaGrantKeyRing` rustdoc now say so. Writing it down
showed an ordering gap: if the backend restarts first, it signs with a key the edge does not know
yet, and those grants fail until the edge rolls. Both the `baukit-core` README and
`deploy/media-grants/README.md` now say to roll the edge first. No code changed.

Gates: `cargo test --manifest-path rust/Cargo.toml -p baukit-core --all-features --
--include-ignored` passes, and `cargo clippy` on the workspace with `--all-features` is clean.

Breaks: none.
