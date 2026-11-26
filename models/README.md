# TLA+ protocol models

Each directory holds one model from Phase 6 of
`docs/quality-security/F-correctness-verification.md`. A model abstracts a protocol that spans processes,
transactions, tabs or devices, where a unit test only ever sees one interleaving. TLC checks every
interleaving of a small instance.

| ID | Model | Code | Findings | Paired code-level test |
|---|---|---|---|---|
| M-01 | [SyncPull](SyncPull/README.md) | `rust/crates/baukit-sync` | F-06, F-07 | P-11 |
| M-02 | [JobLease](JobLease/README.md) | `rust/crates/baukit-jobs` | F-05 | P-10, K-17 |
| M-03 | [BffSessionFamily](BffSessionFamily/README.md) | `rust/crates/baukit-auth/src/browser_session` | F-11, F-27 | C-07 |
| M-04 | [SuiteLink](SuiteLink/README.md) | `rust/crates/baukit-suite/src/services/link.rs` | none | suite link tests |
| M-05 | [SuiteInbox](SuiteInbox/README.md) | `rust/crates/baukit-suite/src/platform/postgres.rs` | none | suite inbox tests |
| M-06 | [ErasureSaga](ErasureSaga/README.md) | `rust/crates/baukit-erasure` | F-40, F-42 | P-14, K-22, K-23 |
| M-07 | [ClientAccountSwitch](ClientAccountSwitch/README.md) | `typescript/packages/data-contracts`, `api-runtime` | F-38, F-53, F-54 | C-02 |
| M-08 | [MembershipCache](MembershipCache/README.md) | `rust/crates/baukit-suite/src/platform/plan_organization.rs` | F-39 | P-15 |

## Config convention

The `models` CI step depends on file names, so keep to them.

- `models/<Name>/<Name>.cfg` models the fixed design. TLC must finish with "No error has been found".
- `models/<Name>/<Name>*_current.cfg` models today's code. TLC must report a violated invariant or
  property. A model may have several, one per finding, so each counterexample stays short.
- Anything else, such as `models/<Name>/residual/*.cfg`, is a reproduction kept for reference. CI does not
  run it. The model's README says what it shows.

When a fix lands, the matching `_current.cfg` keeps describing the old code: it is the evidence that the
model can see the bug. Delete it only if the model changes so that the old behaviour no longer exists in it.

Every config uses small constants so TLC finishes in seconds. The plan's budget is under 5 minutes per
config.

## Running

```sh
sh models/check.sh                 # every model
sh models/check.sh JobLease        # one model
```

`check.sh` uses `$TLA2TOOLS_JAR` (default `~/.local/share/tla/tla2tools.jar`) and `$TLC_WORKERS` (default 4),
runs TLC with `-deadlock` (the models are finite and stop when their bounds are reached), and exits
non-zero if any config gives the wrong verdict.

One config by hand:

```sh
cd models/JobLease
java -XX:+UseParallelGC -cp ~/.local/share/tla/tla2tools.jar tlc2.TLC -workers 4 -deadlock \
  -config JobLease_current.cfg JobLease.tla
```

## Pairing with code

A model is only as good as its match with the code. Every model README lists the code each action stands
for. The paired state-machine test drives the real implementation with the same actions and checks the same
invariants. When you change the code a model maps to, update the model in the same PR.
