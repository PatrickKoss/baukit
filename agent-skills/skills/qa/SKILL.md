---
name: qa
description: Audit a locally runnable application through its real user interface and write an evidence-backed QA and UX report. Use for release-readiness reviews, exploratory testing, usability audits, accessibility checks, visual inspection, or adversarial end-to-end testing. Do not use for code-review-only or unit-test-only requests.
---

# Audit an application as a user

Act as a senior QA engineer, product designer, accessibility reviewer, and adversarial user tester. Test the product as people encounter it, without relying on knowledge of how the code is meant to work.

The goal is to find anything that could make a user think the product is broken, confusing, inconsistent, slow, misleading, difficult to learn, or difficult to recover from. A flow can be technically correct and still be a valid UX finding. Record hesitation and uncertainty instead of rationalizing them away.

Do not stop after the first defect. Work around isolated failures when safe so the rest of the product still gets coverage.

## Set up a safe test environment

1. Read the repository instructions and inspect its routes, architecture, scripts, test utilities, seed data, development accounts, feature flags, and environment configuration.
2. Confirm that the target is a local, preview, or dedicated test environment. Never run destructive tests against production or shared user data.
3. Start the complete application with its intended development workflow. Exercise the real UI, backend, authentication, and persistence layer where they exist.
4. Note the commit or version, runtime, browser or device, viewport, enabled flags, accounts, and test-data volume.
5. Use existing browser automation and accessibility tools when available. Preserve screenshots, traces, console output, requests, and relevant data records as evidence.

Do not change product behavior to make a test pass. Temporary test users, records, seeds, or scripts are allowed when they stay inside the test environment. Keep a record of what was created. Do not claim coverage for a state that was inferred from source code or could not be exercised.

If part of the application cannot run, document the exact blocker and continue with every area that remains testable.

## Map the product before deep testing

Build a feature and state map from both the running UI and the source tree. Include routes or states that normal navigation may hide:

- entry points, authentication, account recovery, onboarding, and sign-out
- primary and secondary navigation, deep links, overlays, dialogs, and multi-step flows
- the product's main workflows and repeated-use workflows
- search, filtering, forms, uploads, editing, deletion, import, export, or sharing where present
- dashboards, history, reports, recommendations, notifications, settings, billing, and account management where present
- loading, empty, error, offline, permission-denied, expired-session, completion, and partial-progress states

Replace this list with the application's actual features in the coverage plan. Do not report absent features as missing unless the product or repository claims to provide them.

## Test with different user behaviors

Use personas that create meaningfully different states:

- A first-time user who needs to understand the product without outside help.
- An impatient user who skips explanations, clicks quickly, and tries to reach the main task at once.
- A confused or non-technical user who changes their mind, dismisses dialogs, uses Back, and takes steps out of order.
- A returning user with realistic history and an expired or resumed session where feasible.
- A power user with substantial data who switches areas quickly and repeats common work.

Add personas required by the product, such as different roles, plans, permissions, locales, organizations, or device capabilities. Use separate accounts or controlled data resets when one persona's state would contaminate another's results.

## Exercise each workflow

For every applicable workflow, test more than one example. Cover the combinations that can expose different behavior:

1. Enter through normal navigation and a direct link.
2. Complete the flow successfully.
3. Make mistakes, omit prerequisites, and submit invalid or partially valid input.
4. Repeat submissions or actions rapidly and watch for duplicate work.
5. Go back, cancel, dismiss, restart, and change an earlier choice.
6. Navigate away, refresh, close and reopen where feasible, then return.
7. Leave mid-flow and check whether the product saves, discards, or explains partial progress.
8. Repeat completed work and use the flow with little data and substantial data.
9. Wait on time-sensitive screens and exercise timeout or expiry behavior where practical.
10. Verify the resulting UI, persisted data, derived totals, and next-step navigation.

For authentication and permissions, cover malformed and incorrect credentials, duplicate registration, loading and duplicate-submit guards, refresh, Back after login or logout, direct access to protected routes, session persistence, expiry, role changes, and stale user state where applicable.

For onboarding or other setup flows, complete, skip, interrupt, revisit, and change earlier answers. Check that skipped context does not leave later screens unexplained.

For text and structured inputs, try values such as empty input, whitespace, different capitalization, punctuation, typos, long text, pasted text, Unicode, emoji, special characters, boundary numbers, and rapid submission. Choose cases that make sense for the field. Judge whether validation is too strict, too permissive, inconsistent, or poorly explained.

## Create meaningful state

Do not audit only an empty account. Prefer normal UI actions, then use safe seeds or direct test-database changes when they save substantial time and do not bypass the behavior under test.

Create a useful spread of states when the product supports them:

- empty or new
- lightly used
- active across several days
- high volume
- repeated failures or poor outcomes
- mostly successful outcomes
- gaps in activity followed by a return
- historical data across day, week, month, and timezone boundaries

Cross-check dashboards, totals, percentages, dates, progress, recommendations, quotas, achievements, and status against the underlying test data where feasible. Look for stale values, impossible combinations, duplicates, partial writes, rounding errors, and actions that are not idempotent.

## Stress state and failure handling

Navigate quickly between unrelated areas. Mix app navigation with browser or device Back, Forward, deep links, refreshes, tab switches, double-clicks, and interrupted requests. Check selected navigation state, scroll position, drafts, cached data, transitions, and stale screens.

Observe asynchronous work under slow and failed network conditions when tooling permits. Users should be able to tell whether the product is waiting, working, done, or failed. Check duplicate-submit protection, input retention, retry behavior, stale content, layout shifts, and recovery after reconnecting.

Find representative empty and failure states. Each empty state should explain why there is no content and offer a relevant next step. Each error should identify the problem in user language, preserve recoverable work, and provide a useful recovery action without exposing internal details.

For state users would expect to keep, verify it after navigation, refresh, close and reopen, logout and login, identity or role switches, and movement between related features. Compare UI state with backend or database state when possible.

## Inspect the interface

Capture and inspect representative screenshots throughout the audit, not only after a visible failure. Include major workflows and meaningful loading, empty, error, completion, overlay, and high-data states.

Compare related screens for:

- alignment, spacing, content width, overflow, clipping, and unexpected layout movement
- typography hierarchy, line length, wrapping, truncation, and readability
- button, field, icon, card, dialog, table, and navigation consistency
- clear action labels, disabled and destructive states, click or touch targets, and work-in-progress feedback
- discoverable interaction, hover, pressed, selected, and focus states
- useful hierarchy and the absence of unnecessary steps or competing content

Test the product's supported viewport and device range. For a responsive web app, include at least a small phone, a typical phone, a tablet-sized viewport, a laptop, and a wide desktop unless repository instructions define another matrix. Check menus, dialogs, forms, tables, charts, fixed actions, virtual keyboards, safe areas, orientation, and scroll behavior where applicable.

## Perform a practical accessibility pass

Use automated checks as a starting point and verify important paths manually. Inspect:

- keyboard access, logical tab order, visible focus, skip behavior, and keyboard-only completion
- semantic controls, headings, landmarks, field labels, errors, instructions, names, roles, and current state
- modal entry, focus containment, Escape or platform dismissal, and focus restoration
- screen-reader names and announcements where the environment supports inspection
- contrast, zoom or large text, reduced motion, disabled-state readability, and non-color cues
- touch-target size, icon-only controls, charts, media alternatives, and interactions that depend on hover or pointer input

Run the repository's existing accessibility suite when available. Do not treat a clean automated scan as proof that the workflow is accessible.

## Review usability and product logic

Ask these questions while testing:

- Does the product show what is happening and whether work was saved?
- Does its language match the user's task rather than its implementation?
- Can users undo, cancel, go back, and recover from mistakes?
- Do similar concepts look and behave the same across the product?
- Does the UI prevent predictable mistakes before submission?
- Does it favor recognition over memory?
- Can experienced users complete frequent work without repeated explanation?
- Does each screen earn its place, and does each question need to be asked now?
- Are defaults sensible, terminology consistent, and important features discoverable?
- Does an error explain both what happened and what the user can do next?

Keep implementation defects separate from product recommendations. Record each meaningful moment of confusion, including questions such as "Did that work?", "Was this saved?", "Why is this disabled?", "How do I get back?", and "What is the difference between these choices?"

## Reproduce and classify findings

For each suspicious behavior:

1. Reproduce it and note whether it is deterministic or intermittent.
2. Try a neighboring route, state, persona, viewport, or input to estimate scope.
3. Capture the smallest useful evidence.
4. Check for related console, network, or persisted-data symptoms.
5. Continue testing nearby functionality.

Classify it as a reproduced defect, intermittent defect, UX concern, product improvement, or unresolved concern. Never invent reproduction steps, evidence, or certainty.

Use these priorities:

- `P0 Critical`: data loss, a serious security or privacy failure, catastrophic failure, or a product that cannot perform its core purpose.
- `P1 High`: a major workflow is blocked or a severe problem affects many users.
- `P2 Medium`: a noticeable problem has a workaround or causes substantial quality loss.
- `P3 Low`: a minor inconsistency, polish defect, or small improvement.

## Maintain coverage and run a final pass

Track each product area as thoroughly tested, partially tested, blocked, or not applicable. A visited screen does not count as tested without meaningful interaction and state coverage.

Before writing the final assessment, compare the coverage list with the routes, components, and feature flags again. Run one more exploratory pass for missed features, accumulated-data failures, persistence defects, and inconsistencies between related workflows. Add every new result to the report.

## Write the audit report

Write the report to `docs/QA_UX_AUDIT.md` unless the user or repository specifies another path. Create `docs/` if needed. Keep raw screenshots and traces in a stable repository location only when the user wants them committed. Otherwise record their local path and do not add large generated artifacts to version control.

Use this report structure:

1. `# QA and UX audit`
2. `## Executive summary`: overall quality, largest functional and UX risks, areas that held up well, unfinished areas, and a release-readiness verdict of `READY`, `READY WITH KNOWN RISKS`, or `NOT READY` with reasons.
3. `## Test environment`: commit or version, environment, commands, runtime, browser or device, viewports, flags, personas, accounts, and approximate test-data volume. Omit secrets.
4. `## Coverage`: a table of product areas, status, states exercised, and gaps or blockers.
5. `## Findings`: findings ordered by priority, then user impact.
6. `## UX improvements`: non-bug changes ranked by user impact.
7. `## Visual polish`: layout, typography, responsive, and component-consistency findings.
8. `## UX friction log`: every meaningful hesitation, the expectation, what the UI communicated, and a concrete improvement.
9. `## Top improvements`: the ten highest-impact changes, or all changes when fewer than ten findings exist.
10. `## Residual risks and blocked coverage`: what remains unknown and why.
11. `## Final regression pass`: areas revisited and anything found during the last pass.

Give each finding a stable ID such as `QA-001`. Include:

- a short, specific title
- type: Functional Bug, UI Bug, UX Issue, Accessibility, Data Integrity, Performance, Consistency, Security or Privacy, or Product Improvement
- priority
- exact route, screen, feature, viewport, and persona when relevant
- preconditions
- numbered reproduction steps
- expected and actual behavior
- evidence with screenshot, trace, console, request, or data references when available
- user impact
- a concrete recommendation
- reproducibility and scope

Do not use "improve UX" as a recommendation. Name the control, state, copy, sequence, or feedback that should change and explain the expected result.

End the command by reporting the audit path, release-readiness verdict, finding counts by priority, blocked coverage, and the most serious issue. Do not fix product defects unless the user also asked for implementation.
