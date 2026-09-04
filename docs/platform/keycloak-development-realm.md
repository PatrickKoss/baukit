# Keycloak development realm

Authenticated backend generation includes a Keycloak 26.7.0 development realm, a policy declaration, and a reconciler. These files are generated only with `--auth oidc`.

## Ownership

The product owns `keycloak/realm.json`. This includes the realm name, registration setting, password rules, clients, redirect choices, and development accounts. The product also owns the limits in `keycloak/realm-policy.json` and the selections in `keycloak/reconcile.json`.

Baukit owns the validation and reconciliation behavior. `scripts/keycloak_policy.py` checks the realm against an explicit `development` or `production` class. It does not infer the class from the realm name. `scripts/reconcile_keycloak.py` accepts only a development reconciliation declaration and updates only the selected realm fields, public clients, and users.

## Policy declaration

Run the generated development check with:

```sh
python3 scripts/keycloak_policy.py --environment-class development
```

The policy declares the accepted password length bounds, whether passwords must exclude the username or email, whether brute-force protection is required, and the minimum TLS setting. Redirect rules set a count limit for each public client, control development loopback HTTP, and list any product-owned application schemes.

The validator checks every client for disabled direct-access grants. Public clients must use PKCE S256. Redirect URI wildcards are limited to a trailing path `/*`; wildcard schemes, hosts, and ports fail. Plain HTTP is accepted only for a loopback address when a development policy allows it.

For a production declaration, set `environmentClass` to `production`, set `minimumTls` to `external` or `all`, and disable development loopback HTTP. Pass `--environment-class production` in the production gate. Production realm JSON should not contain development users or checked-in credentials.

## Reconciliation

`make dev` validates the checked-in realm, starts PostgreSQL and Keycloak, then reconciles the selected development state. The `keycloak-data` volume keeps the realm across container replacement.

The reconciliation declaration contains:

- `realmFields`, a restricted list of top-level realm settings copied from `realm.json`;
- `clients`, the public client IDs and active origins or redirects to merge; and
- `users`, the development usernames to create or update.

The reconciler retains live fields that are absent from the desired representation. It also retains existing origins and redirects when it adds an active development URL. A changed port can be supplied without editing the declaration:

```sh
python3 scripts/reconcile_keycloak.py \
  --client-origin example-web=http://localhost:6173 \
  --client-redirect 'example-web=http://localhost:6173/*'
```

Existing user passwords are left alone. Use `--reset-password USERNAME` for an explicit reset to the corresponding credential in `realm.json`. A newly created user receives its declared development credential.

Fresh generated realms use `test` / `development-password`. Older generated realms used `test` / `password`. Reconciliation preserves the older password unless the product runs `scripts/reconcile_keycloak.py --reset-password test`.

If the configured master administrator cannot authenticate, the script stops Keycloak and creates a random temporary administrator with the pinned container's `bootstrap-admin` command. It uses that account to reconcile the realm and repair the configured administrator. A cleanup step removes the temporary account after success, reconciliation failure, or an interrupt handled by Python. Logs omit passwords, tokens, credentials, and Keycloak response bodies.

## Migration

Existing generated products can adopt this without deleting their Keycloak volume:

1. Add the password policy and brute-force fields required by the product to `realm.json`.
2. Add `realm-policy.json` with an explicit environment class and product limits.
3. Add `reconcile.json` with only the clients, users, and realm fields the development command should manage.
4. Mount `/opt/keycloak/data` on a named volume and run the policy check before reconciliation.
5. Run the reconciler once. Existing unselected realm, client, and user fields remain in place.

The previous `docker compose up -d --wait postgres keycloak` command remains valid. It starts the existing realm without reconciliation. Use `make dev` when the checked-in realm or local ports changed.
