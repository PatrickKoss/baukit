# Evidence for item 18

- Source product files: `/home/patrick/projects/leitbild/keycloak/themes/leitbild/login/login.ftl`, `/home/patrick/projects/leitbild/keycloak/themes/leitbild/login/resources/js/required-validation.js`, `/home/patrick/projects/tiefgang/keycloak/themes/tiefgang/login/login.ftl`, `/home/patrick/projects/tiefgang/keycloak/themes/tiefgang/login/resources/js/required-validation.js`, and `/home/patrick/projects/tiefgang/keycloak/themes/tiefgang/login/resources/js/required-validation.test.mjs`.
- Observed repeated glue: Leitbild and Tiefgang copy Keycloak 26.7.0 `login.ftl` to add required state, linked errors, and first-error focus. Each copy must track upstream template changes.
- Baukit owner: the generated OIDC backend fixture under `keycloak/themes/baukit-accessible`.
- Public types and errors: no runtime API types or product error text. The DOM contract uses owned `baukit-client-error-{control}` IDs and `data-baukit-client-error` markers.
- Product-owned inputs: realm identity, clients, redirects, registration policy, CSS, messages, custom profile fields, and branding.
- Concurrency, failure, privacy, and cleanup: repeated initialization adds no listeners; valid submit posts once; client recovery preserves server and unrelated descriptions; tests require credentials through environment variables; the patch runner deletes only its named disposable Compose volumes and restores temporary realm changes.
- Supported runtimes: Keycloak 26.7.0 and 26.7.1 with inherited `keycloak.v2` markup, tested in Playwright Chromium 151.
- Product adoption change: Leitbild and Tiefgang can inherit `baukit-accessible`, keep their CSS and message bundles, and delete their copied `login.ftl` plus duplicated required-validation script and test after adopting a released Baukit template.
