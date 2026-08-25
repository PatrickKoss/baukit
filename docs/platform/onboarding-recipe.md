# Onboarding recipe

**Status:** Product recipe and acceptance contract, explicitly not a framework.
**Applies to:** Multi-step product onboarding with resumable answers or derived records.

Products own their questions, branch graph, recommendations, generated entities, visuals, and copy. Baukit does not provide a flow engine or a universal answer schema.

## 1. Version the draft

Persist a small envelope such as:

```ts
interface OnboardingDraft<Answers> {
  readonly version: number;
  readonly stepId: string;
  readonly answers: Answers;
}
```

Parse stored JSON as untrusted input. Return explicit `missing`, `valid`, `invalid`, and `version-mismatch` outcomes; do not partially cast malformed data into the current type. A migration may be offered, otherwise quarantine or discard only after product policy/user confirmation.

After restoration, recompute the visible branch graph from validated answers. Resume only at a reachable step; if an earlier answer changes, remove or revalidate answers and derived previews from branches that are no longer reachable.

## 2. Define flow semantics

- Offer explicit resume, start-over, exit, skip (when supported), and completion semantics.
- Confirm before discarding meaningful progress. Exit without discard preserves the latest valid draft.
- Do not silently preselect consequential choices. A prefilled value must be labelled as recommended/default and require intentional confirmation when the consequence warrants it.
- Present one primary progression action per step. Use semantic radio groups and checkboxes, associate errors with fields, announce live validation, and focus the first invalid field after submit.
- Navigation/history must not invent an unreachable step after refresh, browser back, deep link, or a changed branch.

## 3. Complete atomically

Validate the full answer document again at completion. In one product-owned transaction, write every derived entity, its outbox rows, provenance linking those records to the answer/version, and the final onboarding status. Roll back all of them on failure.

Identify a completion attempt with an idempotency key, or define and test a clear duplicate policy. Retrying after a timeout or restart must not create a second set of records. Clear/archive the draft only after committed success; skipping must not leave partial derived entities.

## 4. Acceptance checks

- Refresh/restart on every step and resume the same valid state.
- Exercise missing, malformed, unknown-version, migrated, and quarantined/discarded drafts.
- Change an early branching answer and prove the restored/current step plus hidden answers are revalidated.
- Exercise resume, start-over confirmation/cancellation, exit-with-preserve, skip, and terminal navigation reached by direct load.
- Verify consequential choices have no silent selection and recommended values require the intended confirmation.
- Verify keyboard/touch selection semantics, live validation, and focus-first-invalid.
- Complete all derived writes, provenance, outbox records, and status atomically; inject failure at each write boundary and prove rollback.
- Retry the same completion after same-tick activation, timeout, and restart; prove idempotency or the documented duplicate outcome.
- Verify skip and failed completion do not leave partial product entities and a successful completion retires the draft.
