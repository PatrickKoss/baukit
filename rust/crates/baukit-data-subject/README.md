# baukit-data-subject

Data subject v1 wire types, identifier normalization and Ed25519 DSSE receipts.
The core has no HTTP or database dependencies. The default `server` feature is
currently empty; router, store and worker adapters are planned in D2 and D3.

```toml
[dependencies]
baukit-data-subject = { version = "=0.10.31", default-features = false }
serde_json = "1"
```

Verify a receipt using discovery obtained from the trusted service:

```rust
use baukit_data_subject::{DiscoveryDocument, DsseEnvelope, ReceiptPayload, ReceiptVerifier};

fn verify(discovery_json: &[u8], envelope_json: &[u8])
    -> Result<ReceiptPayload, Box<dyn std::error::Error>>
{
    let discovery: DiscoveryDocument = serde_json::from_slice(discovery_json)?;
    let envelope: DsseEnvelope = serde_json::from_slice(envelope_json)?;
    let verifier = ReceiptVerifier::from_discovery(&discovery)?;
    Ok(verifier.verify(&envelope)?)
}
```

Match the returned service, operation ID, kind and subject reference against the
operation you requested. `ReceiptSigner::from_pkcs8_pem` and `from_pkcs8_der`
load a product's Ed25519 private key. `receipt_key()` returns its current public
discovery entry; `sign()` signs a validated `ReceiptPayload`.

`normalize_request` checks discovery limits and accepted types. Keep its result
transient and store only an HMAC of its serialized normalized body. Raw and
normalized identifier debug output is redacted. Validation errors expose safe
codes without retaining rejected input.

See the [HTTP contract](../../../docs/platform/data-subject-contract.md)
for paths, authentication, status semantics, export parts and exact receipt
encoding. The [public test fixtures](../../../fixtures/data-subject/v1/README.md)
include independent OpenSSL signatures. Never deploy their test private key.
