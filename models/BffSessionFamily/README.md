# M-03 BffSessionFamily

One BFF session family, the IdP's refresh-token family with reuse detection, and an honest browser with
three tabs sharing one cookie jar. Settles F-11.

## What maps to what

| Model | Code (`rust/crates/baukit-auth/src/browser_session/service.rs`) |
|---|---|
| `Begin` | `refresh`: `store.begin()` and `locked_match` (row lock on the cookie's row) |
| `Replay` | `replay`: a rotated row within `GRACE_WINDOW` gets the successor's grace access token and no cookie; after it the family is revoked, locally and at the IdP |
| `IdpRefresh` | `idp.refresh(refresh_token)` inside the open transaction; `InvalidGrant` revokes the family |
| `CommitStore` | `rotate`: insert successor, `mark_rotated`, commit, `Set-Cookie` |
| `CancelAfterRotate` | the request future dropped (client disconnect) or the commit failing after the IdP rotated |
| `GraceLapse` | the 10 s grace passing |
| IdP | rotates on each refresh and revokes the family when a rotated token comes back (Keycloak, Auth0, Okta reuse detection) |

Fixed design: the IdP call and the store run in a detached task that survives cancellation, and a failed
commit retries with the new token. `CancelAfterRotate` does not exist there.

Not modelled: `exchange` and F-27 (a second family per browser), `logout`, `purge`, attacker replays.

## Invariants

- `OneLiveHead`: at most one unrotated row in a live family.
- `NoAccessForRevokedFamily`: no access token is issued for a revoked family.
- `StoreTracksIdp` (fixed config): with no refresh in flight, the stored token is the IdP's current one.
- `HonestNeverLoggedOut`: an honest browser that only retries is never logged out. The plan lists this as a
  liveness property; "never" makes it a safety property, so TLC checks it as an invariant.

## Configs

| Config | Expect |
|---|---|
| `BffSessionFamily.cfg` | pass |
| `BffSessionFamily_current.cfg` | violation of `HonestNeverLoggedOut` |
| `residual/LostResponse.cfg` | violation, not run in CI (see below) |

Tabs t1 to t3, up to 5 rows. Both CI configs assume the browser receives every response it waits for and
retries within the grace window.

## Counterexample on today's code

Tab t2 refreshes. The IdP rotates token 1 to token 2. The client disconnects before commit, so the
transaction rolls back and the row still holds token 1. t2 retries with the same cookie, the server sends
token 1 to the IdP, reuse detection revokes the family, and the user is logged out.

## Residual gap in the fixed design

`residual/LostResponse.cfg` turns on `ResponseLoss` and `GraceCanLapse`. The detached store commits the new
row, but the browser never sees the `Set-Cookie`. Its cookie now points at a rotated row. A retry within 10 s
gets the grace access token but no new cookie; the first refresh after the grace window revokes the family.
The plan's fix does not cover a response lost after commit. Run it with
`tlc2.TLC -deadlock -config residual/LostResponse.cfg BffSessionFamily.tla` from this directory.

## Paired test

C-07 (auth-web `BrowserSessionClient` command model) and a service test that cancels `refresh` after the IdP
call and checks the next refresh succeeds.
