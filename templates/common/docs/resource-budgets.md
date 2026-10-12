# Resource budgets

The product-root `limits.json` file is the shared policy for text, collection, JSON document, row, request body, batch, and whole-number range limits. Its numbers are examples. Replace them with reviewed product limits before release.

Backend code embeds the file at compile time through the domain `limits` module. Web and mobile import the same file during their builds. Keep one policy file so clients can check work before a request while the backend remains authoritative.

The helpers return these stable reason codes at their boundaries: `text_too_long`, `jsonb_too_large`, `too_many_elements`, `too_many_rows`, `body_too_large`, `batch_too_large`, and `out_of_range`. Products own the copy shown for each code.

Increment `limits.json`'s policy version only when its shape or interpretation changes. A value change within the current shape keeps version 1. The parsers still accept maximum-only policies with their existing schema. Maximum-only entries remain positive integers, unless a product explicitly allows zero. Range entries accept zero, require both bounds, and reject negative or fractional bounds, unknown fields, and inverted bounds. Shared range bounds must fit JavaScript's safe integer range, 0 through 9007199254740991.

Declare an inclusive range in `limits.json`:

```json
"ranges": {
  "interval_seconds": { "minimum": 3600, "maximum": 2678400 }
}
```

The Rust parser loads `ranges` as a map of named bounds. The section may be absent in a maximum-only policy. Web and mobile declare the keys in `LIMITS_POLICY_SCHEMA.sections.ranges` and mark each as a range in `rangeKeys`, such as `ranges.interval_seconds`. The TypeScript parser accepts range entries in any declared section; add matching fields to the Rust policy types when using another section.

Use both policy bounds when checking a duration:

```rust
let bounds = limits::limits_policy().ranges["interval_seconds"];
limits::check_range("intervalSeconds", interval.as_secs(), bounds.minimum, bounds.maximum)?;
```

```ts
const { minimum, maximum } = LIMITS_POLICY.ranges.interval_seconds;
checkRange('intervalSeconds', intervalSeconds, minimum, maximum);
```

The Rust core helper uses `u64` for seconds so it accepts `Duration::as_secs()` on every target. Existing length and count helpers use `usize`. TypeScript uses non-negative safe integers. All range checks accept the minimum and maximum themselves. Inverted bounds fail explicitly, even when the measured value is also outside the bounds.

To add a maximum limit, add its key to `limits.json` and `LIMITS_POLICY_SCHEMA.sections`, then call `enforceLimit` with its reason code. A maximum limit that may be zero goes in the schema's `allowZero` list as `section.key`.

A failed client check throws `LimitError` with `reason`, `field`, `measured`, and `allowed`. Range failures throw its subclass `LimitRangeError`, which adds `minimum` and `maximum`; its inherited `allowed` equals `maximum`. Invalid arguments and inverted bounds throw `RangeError` and pass through `enforceLimit`.

Generated backend services preserve domain limit errors. `map_limit_error` maps them to HTTP 400 in the existing error envelope. Range failures carry both bounds:

```json
{
  "error": {
    "code": "out_of_range",
    "message": "The request is invalid",
    "details": {
      "field": "intervalSeconds",
      "measured": 3599,
      "minimum": 3600,
      "maximum": 2678400
    },
    "requestId": "request-id"
  }
}
```

Maximum failures carry `field`, `measured`, and `allowed`. Inverted backend bounds are configuration errors and map to HTTP 500 `internal`. The OpenAPI error schema already permits structured details, so these fields do not change its shape. Generated tests read `tests/fixtures/resource-budget-measurements-v1.json`, the same fixture the Rust and TypeScript toolkit tests use.
