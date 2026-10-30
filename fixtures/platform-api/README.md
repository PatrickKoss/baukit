# Platform API responses

`v1/responses.json` records `/platform/v1` credit and organization response
bodies from the hub repository `schluesselbund` at the commit in `hubCommit`.
Each entry names its source: the route DTO, the error mapping and the hub route
test that asserts it. The Rust and TypeScript contract tests serve these bodies
from a local mock server or parse them directly. Update them when the hub
contract changes.
