# Platform API responses

`v1/responses.json` records `/platform/v1` credit and organization response
bodies from the hub repository `schluesselbund` at the commit in `hubCommit`.
Each entry names its source: the route DTO, the error mapping and the hub route
test that asserts it. The Rust and TypeScript contract tests serve these bodies
from a local mock server or parse them directly. Update them when the hub
contract changes.

`requests` records request bodies Baukit sends. `requests.entitlementTicket`
follows Schlüsselbund C27, which adds `includeBilling` to
`POST /platform/v1/entitlements/tickets`. It was written from the C27 contract
before that change merged, so `hubCommit` does not cover it yet.
