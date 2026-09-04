# Accessible Keycloak login theme spike

**Status:** Decision for a later wave; no theme asset is shipped by this spike.  
**Decision date:** 2026-08-16.

## Decision

Proceed with a later, bounded implementation of an **optional, unbranded,
template-free `keycloak.v2` child theme**. The first implementation should
contain `theme.properties` and one accessibility script, but no copied
FreeMarker templates. A product may use that theme directly or add a child
theme containing its own CSS and message bundles.

This is a conditional **go**. The implementation must be proven against a real
Keycloak container before it is added to the authenticated backend template.
If the required behavior cannot be achieved without copying complete upstream
templates, the implementation is a **no-go**: keep the theme product-owned and
publish only the compatibility and testing recipe. Baukit should not accept the
ongoing upgrade liability of copied `login.ftl` or `register.ftl` files for this
optional feature.

The decision is based on Keycloak 26.7.0. Its inherited `keycloak.v2` markup
already exposes form and field IDs, per-field error containers, server-invalid
state, and live helper regions. A script can enhance those contracts rather
than shadowing the upstream templates.

## Evidence reviewed

### Product implementations

| Implementation  | Overrides and purpose                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Existing verification                                                                                                                                                                                                                    | Maintenance observations                                                                                                                                                                                                                        |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fitness Tracker | `login/theme.properties` inherits `keycloak.v2` and loads `required-validation.js`. Its copied `login.ftl` adds `required=true`, two localized missing-field alert blocks, and preserves the upstream field-level credential error. The script prevents an empty login post, marks both fields required and invalid, appends `aria-describedby`, and focuses the first empty field.                                                                                                              | A fake-DOM Node test covers empty login, both field associations, focus, clearing, and a valid submit.                                                                                                                                   | It overrides only `login.ftl`, but that file is almost the complete upstream page. It does not cover registration. Its server credential error still depends on inherited `field.ftl` behavior and has no explicit credential-error focus test. |
| OpenDialog      | `login/theme.properties` inherits `keycloak.v2`, imports common resources, and loads product CSS plus `opendialog-login.js`. Its copied `login.ftl` forces credential failures into the global message, suppresses field errors, marks login fields required, and enables native validation. Its copied `register.ftl` chiefly removes upstream `novalidate`. The script configures known login/registration fields, creates linked empty-field messages, and focuses a global credential alert. | A Playwright case proves that an empty login stays on the page and displays both messages, then proves that an invalid-credential message remains visible. It does not assert registration, ARIA state, live-region semantics, or focus. | The CSS is entirely product-branded. The JavaScript contains English product copy and assumes standard field names and PatternFly v5 selectors. Both full template copies track Keycloak 26.7 structure.                                        |

Neither product has a `messages_*.properties` overlay. Fitness Tracker obtains
localized missing-field text from inherited Keycloak message keys in its
template. OpenDialog currently places its required-field English text directly
in JavaScript.

The useful overlap is behavioral, not visual:

- login username and password are required;
- empty fields need programmatic required and invalid semantics;
- an inline error must be included in `aria-describedby` without discarding
  existing descriptions;
- errors need a live announcement;
- an empty submit must focus the first invalid field; and
- a server-rendered credential failure must be announced and deliberately
  focused.

Registration support and credential-error presentation differ. Fitness Tracker
does not customize registration and leaves credential errors at the username
field. OpenDialog enhances registration and moves credential failures to a
global alert. Branding, field-specific wording, colors, dark-mode choices, and
realm configuration do not overlap.

### Upstream and Baukit state

The relevant upstream 26.7.0 files are
[`login.ftl`](https://github.com/keycloak/keycloak/blob/26.7.0/themes/src/main/resources/theme/keycloak.v2/login/login.ftl),
[`register.ftl`](https://github.com/keycloak/keycloak/blob/26.7.0/themes/src/main/resources/theme/keycloak.v2/login/register.ftl),
[`field.ftl`](https://github.com/keycloak/keycloak/blob/26.7.0/themes/src/main/resources/theme/keycloak.v2/login/field.ftl),
and
[`user-profile-commons.ftl`](https://github.com/keycloak/keycloak/blob/26.7.0/themes/src/main/resources/theme/keycloak.v2/login/user-profile-commons.ftl).
They provide the stable IDs used by both products. `field.ftl` renders
`input-error-container-{name}`, an `input-error-{name}` message, an
`aria-live="polite"` helper region, and `aria-invalid` for server errors. It
does not connect that message to the input with `aria-describedby`, and its
required argument renders a visual asterisk rather than native `required` or
`aria-required` semantics. The registration profile template similarly knows
which attributes are required but does not put `required` on their controls.

Keycloak recommends extending a built-in theme and using its templates as much
as possible in [Working with themes](https://www.keycloak.org/ui-customization/themes).
Its [upgrade guide](https://www.keycloak.org/docs/latest/upgrading/) explicitly
requires custom template copies to be compared with the new built-in version.
That cost is material here: a copied page shadows upstream changes to imports,
macros, passkeys, WebAuthn, registration profile fields, reCAPTCHA, error
rendering, and CSS classes. A script-only child is still coupled to DOM IDs and
therefore needs compatibility tests, but its copied surface is much smaller.

Baukit currently has two separate pins:

- the [generated development Compose stack](../../templates/backend/compose.yaml)
  uses exact Keycloak `26.7.0`, matching Fitness Tracker;
- the [production Operator base](../../deploy/platform/keycloak/README.md) is
  pinned to `26.7.1`; and
- OpenDialog's Compose files use the floating `26.7` tag. Its locally cached
  image was 26.7.0 during this spike, but the tag does not guarantee that patch.

The [generated realm](../../templates/backend/__auth__/keycloak/realm.json)
does not set `loginTheme`, disables registration, and is the only Keycloak file
mounted by generated Compose. The [generated backend documentation](../../templates/backend/README.md)
describes the realm and development credentials but does not install or select
a theme. No Baukit theme assets exist today.

## Smallest reusable design

The later implementation should first prove this structure:

```text
keycloak/themes/
  baukit-accessible/login/
    theme.properties              # parent=keycloak.v2; accessibility script
    resources/js/accessibility.js
  product/login/                  # optional and product-owned
    theme.properties              # parent=baukit-accessible
    messages/messages_*.properties
    resources/css/product.css
```

The base script should:

1. Enhance only the inherited login and registration forms. Discover required
   registration controls from the upstream required marker/group structure;
   do not encode one product's profile schema.
2. Set native `required` plus `aria-required="true"`, while preserving WebAuthn
   autocomplete and hidden-username flows.
3. On submit, expose errors for every empty required control, set
   `aria-invalid="true"`, append (never replace) the owned error ID in
   `aria-describedby`, announce through an owned live region, prevent the
   invalid post, and focus the first invalid control.
4. On input, remove only the client error and association it owns. Preserve
   server errors and unrelated descriptions.
5. On a server response, associate inherited `input-error-{name}` content with
   its invalid control, make the error reliably live, and focus the first
   server-invalid control. If Keycloak renders only a global danger alert,
   make it an alert, temporarily focusable, and focus it instead.
6. Be idempotent if initialization runs more than once and do nothing on other
   Keycloak pages.

Required-field text should use the browser's localized constraint-validation
message in the template-free base. Products may override inherited Keycloak
messages in their child theme and add their own styling; product overlays must
not replace or accidentally omit the base script. The integration fixture must
prove the resource ordering and parent lookup rather than relying on an
untested `theme.properties` assumption.

This design intentionally avoids a generic design system for Keycloak and does
not promise support for arbitrary custom user-profile widgets. A custom
control outside the tested inherited `keycloak.v2` forms remains product-owned.

## Deliverables for the later wave

If the real-container proof succeeds, ship all of the following together:

- the unbranded child theme and focused JavaScript unit tests;
- real-browser tests against the exact supported Keycloak image;
- an optional product-child example proving CSS and message overlays while the
  base accessibility script still loads;
- a generated realm reference, `"loginTheme": "baukit-accessible"`, for the
  flavor that includes the asset;
- a generated read-only Compose mount of `./keycloak/themes` at
  `/opt/keycloak/themes`;
- generated documentation explaining direct use, product-child inheritance,
  cache/restart behavior, registration enablement, and production packaging;
  and
- a compatibility note naming the exact tested Keycloak patch and the
  selectors/markup contracts on which the script relies.

The realm selection and Compose mount must be tested as generated output, not
only documented snippets. Production delivery for the Operator base also needs
an explicit, immutable theme packaging/mount mechanism; a development bind
mount is not a production deployment design.

## Acceptance tests

Run JavaScript unit tests for DOM-state transitions and Playwright against a
real Keycloak container. The real-browser suite is the release gate and must
cover:

1. **Empty login:** submitting empty username and password does not post,
   exposes both errors, sets `required`, `aria-required`, and
   `aria-invalid="true"`, appends valid `aria-describedby` references, makes
   the errors live, and focuses username.
2. **Empty registration:** with registration enabled, submitting the inherited
   empty registration form exposes every required standard field and focuses
   the first one. Password and confirmation are included. Every referenced
   description ID exists.
3. **Server credential error:** submitting a real username with a wrong
   password returns the Keycloak error, gives it alert/live semantics,
   associates field-level output when present, and moves focus to the chosen
   error target. The test must assert focus, not only visibility.
4. **Recovery:** typing into a field clears only its client-side error; other
   field errors, server errors, and pre-existing `aria-describedby` tokens
   remain intact. A valid login still posts exactly once.
5. **Variant flows:** username-hidden and conditional-WebAuthn login validate
   only the visible controls and preserve Keycloak's autocomplete value.
6. **Child overlay:** a product child supplies neutral test CSS and one message
   override while all base behavior still runs. Repeat at least one case with
   Keycloak internationalization enabled.
7. **Generated wiring:** scaffold the authenticated fixture, assert the theme
   tree, realm reference, and read-only Compose mount, then execute login and
   registration cases through that fixture.

The initial support statement should be **Keycloak 26.7.0 only** because that
is the generated Compose pin. Before the asset is used with Baukit's 26.7.1
Operator base, run the same suite against 26.7.1 and record it as supported.
Every Keycloak patch/minor upgrade must inspect the inherited form contracts and
rerun the real-browser suite. Do not use a floating `26.7` tag in the test
matrix or compatibility statement.

## Content boundary

The Baukit asset must never embed:

- product names, logos, copy, colors, fonts, or design tokens;
- product realm names, client IDs, redirect URIs, or origins;
- usernames, e-mail addresses, passwords, client secrets, administrator
  credentials, or other test/product credentials; or
- product-specific registration fields, validation policy, analytics, or
  business logic.

Only neutral accessibility behavior and test-only neutral fixtures belong in
Baukit. Product CSS, message choices, and realm identity remain overlays owned
by each product.

## Outcome

The implementation proof succeeded without copied FreeMarker templates. The generated OIDC fixture now includes `baukit-accessible`, a script-only `keycloak.v2` child, and `baukit-accessible-test`, a neutral child used only to verify inheritance. The realm selects the base theme and the reconciler owns that field. Compose mounts the complete theme tree read-only.

The fake-DOM suite covers client error creation and recovery, server field and global errors, registration discovery, native invalid events, idempotence, hidden controls, autocomplete preservation, and unrelated pages. The generated Playwright suite passed against the exact `quay.io/keycloak/keycloak:26.7.0` and `quay.io/keycloak/keycloak:26.7.1` images. Both patches are supported.

The tested inherited contracts are `#kc-form-login`, `#kc-register-form`, standard control IDs, `input-error-{name}`, and the PatternFly 5 or 6 form-group and required-marker classes. Keycloak `login.ftl`, `login-username.ftl`, `login-password.ftl`, and `register.ftl` remain inherited. Any Keycloak upgrade must inspect these contracts and rerun both exact-image browser suites before support changes.
