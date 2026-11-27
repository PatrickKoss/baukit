# M-03 BffSessionFamily

One BFF session family, the IdP's refresh-token family with reuse detection, and an honest browser with
three tabs sharing one cookie jar. Settles F-11.

## What maps to what

| Model | Code (`rust/crates/baukit-auth/src/browser_session/service.rs`) |
|---|---|
| `Begin` | `refresh`: `store.begin()` and `locked_match` (row lock on the cookie's row) |
| `Replay` | `replay`: a rotated row within `GRACE_WINDOW` gets the successor's grace access token and no cookie; after it the family is revoked, locally and at the IdP. The model does not include the unused-head path, see below |
| `IdpRefresh` | `idp.refresh(refresh_token)` inside the open transaction; `InvalidGrant` revokes the family |
| `CommitStore` | `rotate`: insert successor, `mark_rotated`, commit, `Set-Cookie` |
| `CancelAfterRotate` | the request future dropped (client disconnect) or the commit failing after the IdP rotated |
| `GraceLapse` | the 10 s grace passing |
| IdP | rotates on each refresh and revokes the family when a rotated token comes back (Keycloak, Auth0, Okta reuse detection) |

Fixed design, which is the code since commit `fc8e89c7`: `refresh` runs the IdP call and the store in a
`tokio::spawn` task that survives cancellation, and a failed commit retries with the new token up to
`STORE_ATTEMPTS` (3) times. `CancelAfterRotate` does not exist there.

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
| `residual/LostResponse.cfg` | violation of `HonestNeverLoggedOut`, not run in CI (see below) |

Tabs t1 to t3, up to 5 rows. Both CI configs assume the browser receives every response it waits for and
retries within the grace window.

## Counterexample on the current design

`Design = "current"` models the code before `fc8e89c7`. Tab t2 refreshes. The IdP rotates token 1 to token 2. The client disconnects before commit, so the
transaction rolls back and the row still holds token 1. t2 retries with the same cookie, the server sends
token 1 to the IdP, reuse detection revokes the family, and the user is logged out.

## Lost response after commit

`residual/LostResponse.cfg` turns on `ResponseLoss` and `GraceCanLapse`. The detached store commits the new
row, but the browser never sees the `Set-Cookie`. Its cookie now points at a rotated row. In the model, a
retry within the 10 s grace gets the grace access token but no new cookie, and the first refresh after the
grace window revokes the family. TLC still reports that violation, because the spec does not model the
recovery path below. Run it with
`tlc2.TLC -deadlock -config residual/LostResponse.cfg BffSessionFamily.tla` from this directory.

The code now handles this case. In `replay`, when the presented row's successor is an unused head,
`rotate_unused_head` refreshes that head at the IdP and issues a new cookie instead of revoking the family.
`is_unused_head` requires all of the following:

- the successor is not rotated and not revoked;
- its grace window has passed;
- at most `LOST_RESPONSE_CAP` (5 minutes) has passed since the successor was created;
- it is within its idle and absolute expiry.

`lost_refresh_response_is_recovered_by_the_previous_cookie` in `refresh_tests.rs` checks a retry 30 s
after the lost response.

What is still a residual:

- A browser that retries with the old cookie more than 5 minutes after the lost response is logged out.
  The family is revoked, locally and at the IdP.
- Two lost responses in a row. After the first recovery the old cookie's successor is rotated, so it is no
  longer an unused head, and a retry past its grace window revokes the family.
- Once the browser has used the new head, the old cookie revokes the family, as reuse detection intends.
  `previous_cookie_after_the_head_was_used_still_revokes_the_family` checks this.

## Paired test

C-07 (auth-web `BrowserSessionClient` command model) and a service test that cancels `refresh` after the IdP
call and checks the next refresh succeeds.
