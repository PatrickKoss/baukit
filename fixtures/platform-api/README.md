# Platform API responses

`v1/responses.json` records `/platform/v1` credit and organization response
bodies from the hub repository `schluesselbund` at the commit in `hubCommit`.
Each entry names its source: the route DTO, the error mapping and the hub route
test that asserts it. The Rust and TypeScript contract tests serve these bodies
from a local mock server or parse them directly. Update them when the hub
contract changes.

`requests` records request bodies Baukit sends. `requests.entitlementTicket`
follows Schlüsselbund C27 (commit 823f394, merged as aa3204d), which adds
`includeBilling` to `POST /platform/v1/entitlements/tickets`. The claims the hub
issues for it are recorded in the hub's `fixtures/platform-tickets/v1/billing.json`.
