# Browser QA configuration backport

## Source product files

- `/home/patrick/projects/leitbild/web/e2e/qa.config.ts`
- `/home/patrick/projects/leitbild/web/e2e/playwright.config.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/geometry.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/qa-axe.spec.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/qa-keyboard.spec.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/qa-overlay-dismiss.spec.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/qa-route-state.spec.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/qa-scroll.spec.ts`
- `/home/patrick/projects/leitbild/web/e2e/tests/qa-submit-guards.spec.ts`

## Observed failure or repeated glue

Leitbild copied the generated browser specs to test authenticated screens, per-case API data,
regular-expression headings, route-local selectors, and multi-field forms. Its Playwright server
also needed an explicit working directory. The copy removed the delayed first-load assertion and
weakened account, inert-background, scroller, and console checks.

## Baukit owner

The web template owns the configurable Playwright specs and their synthetic generated fixture.

## Public types and errors

`QaRoute`, `QaOverlay`, `QaSubmitTarget`, `QaRouteState`, and `QaProtectedRoute` accept case-specific
authentication or API inputs. `QaAuthentication`, `QaStorageEntry`, and `QaSubmitField` describe
synthetic login state and form values. An authenticated case without configured state throws a
setup error before navigation.

## Product-owned inputs

Products own routes, selectors, accessible names, roles, API patterns and response bodies,
authentication storage format, account fixtures, and which routes need scroll checks.

## Required cases

- Concurrency: submit tests still issue two same-task activations and expect one result.
- Failure: delayed first load, expired authentication, invalid form focus, route-state recovery,
  bounded keyboard search, overflow diagnostics, and missing second-account skips remain visible.
- Privacy: generated login state uses a synthetic token. Products must not copy credentials or
  private production responses into QA configuration or diagnostics.
- Cleanup: account-switch tests clear cookies and browser storage between identities. Playwright
  owns preview-server shutdown and browser contexts.

## Supported runtimes

Playwright 1.62.1 on generated Node 24 projects, using desktop and mobile Chromium and WebKit. The
geometry audit retains the 320, 1023, and 1024 CSS-pixel breakpoint cases.

## Product adoption change

Leitbild can replace its modified `web/e2e/tests/qa-*.spec.ts`, `geometry.ts`, and Playwright server
setup with newly generated Baukit copies. It keeps product routes, selectors, fixtures,
`critical-path.spec.ts`, `api-access-tokens.spec.ts`, and `qa-product-regressions.spec.ts`.
