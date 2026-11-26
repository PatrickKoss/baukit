# Data-subject API accepts only client-credentials tokens

`baukit-data-subject` now refuses a token unless its `sub` is the calling client's id
(Auth0 service tokens use `<client>@clients`) and it carries no user grants. Before, a
user's token issued to an allowlisted client with the right scope was accepted.

Clerk and WorkOS machine tokens already set `sub` to the client id. Auth0 sets
`<client>@clients`. Keycloak service accounts set `sub` to the service account's user
id, so add a hardcoded-claim mapper to the client:

1. Clients, then the privacy client, then Client scopes, then the dedicated scope.
2. Add a "Hardcoded claim" mapper with claim name `sub`, the client id as value,
   type String, and "Add to access token" on.

Check by calling `GET /api/v1/data-subject/erasures/<random uuid>` with a fresh service token. A
`404` means the token passed. A `403 data_subject_client_not_allowed` means `sub` is
still wrong.
