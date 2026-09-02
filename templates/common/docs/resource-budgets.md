# Resource budgets

The product-root `limits.json` file is the shared policy for text, collection, JSON document, row, request body, and batch limits. Its numbers are examples. Replace them with reviewed product limits before release.

Backend code embeds the file at compile time through the domain `limits` module. Web and mobile import the same file during their builds. Keep one policy file so clients can reject oversized work before a request while the backend remains authoritative.

The helpers return these stable reason codes at their boundaries: `text_too_long`, `jsonb_too_large`, `too_many_elements`, `too_many_rows`, `body_too_large`, and `batch_too_large`. Products own the copy shown for each code.

Increment `limits.json`'s policy version only when its shape or interpretation changes. A value change within the current shape keeps version 1. The generated parsers reject unknown fields, unsupported versions, zero limits, and invalid numeric values.
