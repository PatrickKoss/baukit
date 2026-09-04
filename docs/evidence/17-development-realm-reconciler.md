# Evidence for item 17

- Source product files: `/home/patrick/projects/redemut/deploy/keycloak/configure-dev-realm.sh`, `/home/patrick/projects/redemut/Makefile`, and `/home/patrick/projects/redemut/deploy/docker-compose.yml`.
- Observed failure or repeated glue: Keycloak imports a realm only into fresh storage. A retained volume misses later port, client, user, and realm changes.
- Baukit owner: generated development Compose and the dependency-free authenticated backend reconciler.
- Public types and errors: `keycloak/reconcile.json` selects realm fields, public clients, users, origins, and redirects; command failures return exit 1 with redacted diagnostics.
- Product-owned inputs: the desired realm JSON, selection list, active URLs, administrator name, development users, and any explicit password reset.
- Concurrency, failure, privacy, and cleanup: one development invocation is expected at a time; repeated runs make no further updates; recovery cleanup runs after success, failure, and handled interrupts; logs omit secrets, tokens, credentials, and response bodies.
- Supported runtimes: Python 3 standard library, Docker Compose, and Keycloak 26.7.0.
- Product adoption change: Redemut can delete `deploy/keycloak/configure-dev-realm.sh` and point `dev-keycloak-user` at the generated reconciler after adopting the release.
