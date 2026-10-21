# Data subject v1 test fixtures

All keys here are public test keys. The private seed is the byte sequence 0
through 31. Never deploy it or use it to protect real data. No fixture contains
a real person's identifiers or production key material.

`discovery.json` advertises the test public key. `test-only-private.pem` and
`.der` encode the same Ed25519 PKCS#8 v1 private key. `test-only-public.pem`
and `.der` encode its SPKI public key. The key ID is lowercase hexadecimal
SHA-256 of `test-only-public.der`, not of the PEM text or raw public key.

`cases.json` labels every envelope's expected result. `completed`, `notFound`,
`restricted` and `export` are canonical positive cases. Each has a
`.payload.json` containing exactly the signed bytes and a `.pae` containing
the Ed25519 input. Payload files have no trailing newline. The export case
signs the size and SHA-256 of `export-part-1.json` and `export-part-2.json`.
The part bodies illustrate a service-defined record format.

All positive signatures were produced independently of the Rust code by
[`generate.py`](./generate.py), using Python's standard JSON and base64 libraries
and the OpenSSL CLI. `openssl-noncanonical` deliberately reverses the payload
field order and uses indented JSON with a final newline. Its signature must
verify over those received bytes even though the Rust signer emits compact JSON.
The Rust tests reproduce the four canonical envelopes byte for byte and verify
the independent noncanonical envelope.

Reproduce the fixtures from the repository root:

```sh
python3 fixtures/data-subject/v1/generate.py
```

The script constructs DSSE v1 PAE using decimal UTF-8 byte lengths and ASCII
spaces. For each positive case it runs this command, for example:

```sh
cd fixtures/data-subject/v1
openssl pkeyutl -sign -rawin -inkey test-only-private.pem \
  -in completed.pae -out completed.sig
openssl pkeyutl -verify -rawin -pubin -inkey test-only-public.pem \
  -in completed.pae -sigfile completed.sig
rm completed.sig
```

The signature is standard base64 in the envelope. The generator uses no Rust
output or Baukit signing library. It then creates three negative envelopes:

| File | Expected rejection |
| --- | --- |
| `tampered-payload.envelope.json` | A category count changes after signing; signature verification fails. |
| `wrong-payload-type.envelope.json` | The media type is not the receipt media type. |
| `unknown-key.envelope.json` | The signing ID is absent from discovery. |

The exact encoding and canonical field order are in the
[contract](../../../docs/platform/data-subject-contract.md).
