# Verification targets

Each target runs one `scripts/ci-gate.sh` step, the same command its CI job runs. Tools come from
`mise.toml` (`make toolchain`); the TLA+ jar comes from `deploy/tools.toml` and can be overridden
with `TLA2TOOLS_JAR`.

| Target | Step | What it proves |
|---|---|---|
| `make kani` | `kani` | Every `#[kani::proof]` in a crate with `cfg(kani)` code holds for all inputs within the bounds its doc comment states. Also fails when a harness has no `Bounds:` line. |
| `make kani-full` | `kani-full` | The same for every crate at once. Runs nightly. |
| `make verus` | `verus` | The Verus proofs in `rust/verus` (V-01 to V-03, below), the proptests that compare each twin with production, and the Kani harnesses that prove the HLC and bucket twins equal to production. |
| `make models` | `models` | `models/check.sh`: each `models/<Name>/<Name>.cfg` passes TLC (deadlock checking off), and each `<Name>*_current.cfg`, which reproduces a bug in today's code, reports a violation. |
| `make fuzz` | `fuzz` | No fuzz target crashes within 60 s. |
| `make fuzz-long` | `fuzz-nightly` | The same for `FUZZ_SECONDS` per target (default 1800), growing the corpus. |
| `make migrations-check` | `migrations` | New migrations pass squawk, cross-crate order matches `rust/migrations.toml`, every new file reruns cleanly, and the previous release's store tests pass on HEAD's schema. |
| `make workflows-audit` | `workflows` | actionlint and zizmor (auditor persona) report nothing of low severity or above, and every OIDC job is gated and publish-only. |
| `make vectors` | `vectors-regenerate` | Rewrites every `fixtures/**/generated-v1.json` from `scripts/vector-generators.toml`. |
| `make vectors-check` | `vectors` | The committed vectors match what the generators produce. |
| `make proofs-list` | none | Prints the Kani harness table with each harness's bounds. |
| `make verify` | kani, models, vectors, migrations, workflows | The pre-push set. `make ci` includes it. |

A proof covers only what its harness or model states. Read the bounds column before relying on one.

## Verus

`rust/verus` holds verified twins of three production kernels. Verus proves each theorem for
traces of any length, where Kani stops at its unwind bound.

| ID | Twin | Production | Proven |
|---|---|---|---|
| V-01 | `hlc.rs` `replay` | `baukit_sync::hlc` `now_step`, `observe_step`, `HybridLogicalClock::{now, observe}` | Timestamps strictly increase and exceed the starting state. The final `wall * 1000 + counter` is at most `max(start, (max_physical + max_drift) * 1000 + 999)` plus the number of calls, so the wall clock passes `max_physical + max_drift` only by counter roll-over, 1 ms per 1000 calls. |
| V-02 | `bucket.rs` `replay` | `baukit_ratelimit::kernels::bucket` `refill`, `admit`, and the entry update in `InMemoryRateLimitStore::check_at` | For one key with a fixed scaled quota, starting full at `t0`: admitted cost is at most `capacity + rate * (t - t0)`, `t` the latest request time. Out-of-order times count as no refill, as in production. |
| V-03 | `pull.rs` `page_progress_and_no_skip` | `baukit_sync::pull::merge_pull_page` | Given table reads that honour `(revision, id) > after`, `revision <= hi` and `LIMIT limit` with `limit >= 1`: the page sets `more` and moves `next` strictly past `after`, or clears `more` and returns `after_revision(hi)`. Every window row is in the page or above `next`. |

The twins use the production integer types and the same checked and saturating arithmetic,
written as explicit range checks Verus can reason about. Two links keep them honest:
`rust/verus/equivalence` runs proptests over random and edge-case inputs for all three twins, and
Kani harnesses that prove twin and production equal: HLC `now_step`, `observe_step`, `encode` and
`decode` over unconstrained `i64`; bucket `refill` and `admit` at rates 0, 1, 7 and `u64::MAX`
with the other inputs `<= 15` or `u64::MAX`. The pull merge has no Kani harness: CBMC needs more
than 8 GB on the production `Uuid` byte comparison and sort even for two one-row tables, so its
link is the proptest (4096 cases).

### Layout

The plan offered two layouts. (a) puts the production kernels inside `verus!` and depends on
`vstd`. vstd's erased build compiles on MSRV 1.95 and passes `cargo deny`, so (a) was possible.
I chose (b), a twin outside the main workspace, because (a) adds a `0.0.0` pre-release proc macro
(`verus_builtin_macros` on a forked `syn`) to the build of every product that uses `baukit-sync`
or `baukit-ratelimit`, and moves weekly with each Verus release. The twin has its own
`[workspace]` and `Cargo.lock`, so none of that reaches `rust/Cargo.lock`.

### Running it

`make verus` runs the gate step. Verus comes from `mise.toml` (`http:verus`, release zip pinned
by sha256, z3 bundled inside it) and needs the pinned rust 1.100.0 toolchain. For the proofs alone:

```sh
cargo verus verify --manifest-path rust/verus/Cargo.toml
```

A Verus release bump changes the `vstd` pin in `rust/verus/Cargo.toml`; `cargo verus new` prints
the matching one.

## Kani harnesses

`python3 scripts/list-proofs.py` generates this table from the harness doc comments, and
`make proofs-list` prints it. `scripts/ci-gate.sh --step kani` fails when a harness has no
`Bounds:` sentence. The V-01 and V-02 rows are the equivalence harnesses in
`rust/verus/equivalence` that tie each Verus twin to production code.

| ID | Crate | Harness | Bounds |
|---|---|---|---|
| K-07 | baukit-auth | `well_formed_bytes_matches_the_token_shape` | inputs of 0 to 8 bytes, a 4-byte secret, unwind 9. |
| K-05 | baukit-auth | `refresh_allowed_spaces_attempts_by_the_cooldown` | every non-zero `u64` cooldown, optional `u64` last attempt and `u64` now, no loops. |
| K-06 | baukit-auth | `claims_time_valid_matches_exact_window` | every `i64` exp, nbf, now and skew, no loops. |
| K-08 | baukit-auth | `constant_time_eq_matches_equality` | two byte strings of 0 to 8 bytes, unwind 9. |
| K-13 | baukit-core | `check_expiry_bounds` | all `u64` triples, no loops. `Ok` implies `now < expires <= now + max_ahead` with the sum taken in `u128`. |
| K-14 | baukit-core | `parse_expires_accepts_only_canonical_text` | ASCII strings of 0 to 4 bytes, unwind 8. An accepted expiry is non-empty decimal digits without a leading zero whose value matches the text. |
| K-14 | baukit-core | `base64url_anchor_rejects_foreign_bytes` | ASCII strings of 0 to 4 bytes, unwind 8. Every accepted string is non-empty and uses only the base64url alphabet, so `\n`, `=` and `+` never pass; the round-trip check itself runs in the unit tests. |
| K-24 | baukit-data-subject | `placement_keeps_every_part_within_the_limit` | part, record and limit below 2^20 bytes, no loops. |
| K-03 | baukit-egress | `ipv4_public_matches_spec` | every `u32` address, no loops. |
| K-03 | baukit-egress | `ipv6_public_matches_spec` | every `u128` address, unwind 17. |
| K-03 | baukit-egress | `public_only_permits_matches_spec` | every `u32` address, no loops. |
| K-04 | baukit-egress | `ipv4_mask_has_prefix_leading_ones` | prefix 0 to 32, no loops. |
| K-04 | baukit-egress | `ipv6_mask_has_prefix_leading_ones` | prefix 0 to 128, no loops. |
| K-23 | baukit-erasure | `deletion_outcome_ok_only_for_2xx_or_missing_404` | every `u16` status, both body answers, no loops. |
| K-22 | baukit-erasure | `fence_matches_truth_table` | every fence row and `i64` login time, no loops. |
| K-22 | baukit-erasure | `replay_matches_truth_table` | all four boolean inputs, no loops. |
| K-18 | baukit-integrations | `token_length_gate_admits_only_43_characters` | inputs of 0 to 44 characters, no loops. |
| K-17 | baukit-jobs | `stale_attempt_never_holds_the_lease` | two worker ids, attempts below 4, `u8` instants (the predicate is generic over the instant type, so this covers ordering without jiff's internals). Worker ids are 8 bytes, so the string compare unwinds at most 9 times. |
| K-12 | baukit-ratelimit | `window_bounds_contain_now` | `now` is an `i16` and `duration` is `u8 + 1`, both widened to `i64`. Full `i64` and `i32`/`u16 + 1` each exceeded 10 minutes in CBMC. The proptest `window_properties::window_bounds_contain_now` samples all of `i64`. No loops. |
| K-11 | baukit-ratelimit | `admit_keeps_tokens_in_bounds` | every `u64` input, no loops. |
| K-01 | baukit-ratelimit | `forwarded_selection_ignores_client_controlled_entries` | lists of 0 to 6 IPv4 or unparseable entries, hops 0 to 4, unwind 8. |
| K-02 | baukit-ratelimit | `forwarded_selection_is_peer_or_trusted_entry` | lists of 0 to 6 IPv4 or unparseable entries, hops 0 to 4, unwind 8. |
| K-19 | baukit-ratelimit | `ip_bucket_groups_exactly_by_prefix` | every pair of 128-bit addresses, prefix 0 to 128, no loops. |
| K-15 | baukit-suite | `signature_window_is_symmetric_without_overflow` | every `i64` timestamp and now, no loops. |
| K-16 | baukit-suite | `native_scheme_is_own_or_opted_in_dev` | ASCII schemes of 0 to 7 bytes, own scheme of 0 to 3 bytes, unwind 8. |
| K-21 | baukit-suite | `an_encoded_segment_cannot_leave_its_slot` | 3 bytes, unwind 12. |
| K-21 | baukit-suite | `any_single_char_segment_stays_in_its_slot` | one char, unwind 12. |
| K-20 | baukit-suite | `an_organization_subject_is_always_the_token_organization` | every optional token organization and named `u128` UUID, no loops. |
| K-09 | baukit-sync | `now_step_advances_within_bound` | components within 4 of zero or their maximum, no loops. |
| K-09 | baukit-sync | `observe_step_respects_drift_bound` | components within 4 of zero or their maximum, drift below 4, no loops. |
| K-10 | baukit-sync | `encode_round_trips_and_compare_is_lexicographic` | components within 4 of zero or their maximum, no loops. |
| V-02 | verus | `admit_equivalent_rate_zero` | rate 0; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`. |
| V-02 | verus | `admit_equivalent_rate_one` | rate 1; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`. |
| V-02 | verus | `admit_equivalent_rate_seven` | rate 7; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`. |
| V-02 | verus | `admit_equivalent_rate_max` | rate `u64::MAX`; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`. |
| V-01 | verus | `now_step_equivalent` | none, straight-line code over unconstrained `i64`. |
| V-01 | verus | `observe_step_equivalent` | none, straight-line code over unconstrained `i64`. |
| V-01 | verus | `codec_equivalent` | none, straight-line code over unconstrained `i64`. |

## Known gaps

- `scripts/migrations-old-binary.sh` runs the `[old_binary] tests` from `rust/migrations.toml`
  against the previous release's binary. The list is empty because no published release read
  `BAUKIT_TEST_DATABASE_URL` when it was added. Until it is filled, the double-apply and Postgres
  suites cover the expand migrations, and no old binary runs against the new schema.
- Suite receivers still accept live deliveries signed only with v1. They refuse `x-suite-replay`
  without `x-suite-signature-v2`. Refusing every v1-only delivery waits until all peers send v2.
  See `docs/migrations/suite-correctness-hardening.md`.
- The pull page merge has no Kani equivalence harness. CBMC runs out of 8 GB on the production
  `Uuid` compare and sort even for two one-row tables. `merge_matches_production` in
  `rust/verus/equivalence/src/pull.rs` compares twin and production over 4096 proptest cases
  instead. Verus proves V-03 itself without a bound.
- K-12 runs at reduced width. `now` is an `i16` and `duration` a `u8 + 1`, because wider types
  took more than 10 minutes in CBMC. The proptest `window_properties::window_bounds_contain_now`
  samples the full `i64` range.
- K-18 proves the length gate only. CBMC cannot bound the base64url decoder, so the
  `oauth_state_parse` fuzz target checks canonical acceptance at 43 characters.
- K-21 works per byte. The harnesses cover every 3-byte string and every single `char`. The
  encoder maps bytes one at a time, so longer segments follow, and whole-segment proptests in
  `baukit-suite` cover them directly.
