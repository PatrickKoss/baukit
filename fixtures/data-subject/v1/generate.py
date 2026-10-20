#!/usr/bin/env python3
"""Reproduce public test-only DSSE vectors with Python and OpenSSL, without Rust."""

import base64
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
PAYLOAD_TYPE = "application/vnd.baukit.data-subject.receipt.v1+json"


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def write_json(name, value):
    (ROOT / name).write_bytes(compact(value) + b"\n")


def openssl(*args):
    subprocess.run(["openssl", *args], cwd=ROOT, check=True, capture_output=True)


def envelope(name, body):
    pae = (
        b"DSSEv1 " + str(len(PAYLOAD_TYPE.encode())).encode() + b" "
        + PAYLOAD_TYPE.encode() + b" " + str(len(body)).encode() + b" " + body
    )
    (ROOT / f"{name}.payload.json").write_bytes(body)
    (ROOT / f"{name}.pae").write_bytes(pae)
    openssl("pkeyutl", "-sign", "-rawin", "-inkey", "test-only-private.pem",
            "-in", f"{name}.pae", "-out", f"{name}.sig")
    signature = (ROOT / f"{name}.sig").read_bytes()
    (ROOT / f"{name}.sig").unlink()
    result = {
        "payloadType": PAYLOAD_TYPE,
        "payload": base64.b64encode(body).decode(),
        "signatures": [{"keyid": KEY_ID, "sig": base64.b64encode(signature).decode()}],
    }
    write_json(f"{name}.envelope.json", result)
    return result


# PUBLIC TEST KEY. Deterministic bytes 0..31 are not a production secret.
private_der = bytes.fromhex("302e020100300506032b657004220420") + bytes(range(32))
(ROOT / "test-only-private.der").write_bytes(private_der)
(ROOT / "test-only-private.pem").write_text(
    "-----BEGIN PRIVATE KEY-----\n" + base64.b64encode(private_der).decode()
    + "\n-----END PRIVATE KEY-----\n"
)
openssl("pkey", "-in", "test-only-private.pem", "-pubout", "-out", "test-only-public.pem")
openssl("pkey", "-in", "test-only-private.pem", "-pubout", "-outform", "DER",
        "-out", "test-only-public.der")
KEY_ID = hashlib.sha256((ROOT / "test-only-public.der").read_bytes()).hexdigest()
service = {"name": "fixture-service", "version": "1.0.0"}
write_json("discovery.json", {
    "contractVersion": "1",
    "service": service,
    "operations": ["erasure", "export"],
    "identifierTypes": ["email", "accountId", "customerId", "phone", "x-fixture-id"],
    "scopes": {"erasure": "data-subject.erase", "export": "data-subject.export"},
    "limits": {"maxIdentifiers": 16, "maxIdentifierLength": 256, "maxExportPartBytes": 8388608},
    "receiptKeys": [{"keyId": KEY_ID, "algorithm": "ed25519",
                     "publicKeyPem": (ROOT / "test-only-public.pem").read_text(),
                     "status": "current"}],
})

completed = None
cases = []
for index, status in enumerate(["completed", "notFound", "restricted", "export"], 1):
    exported = status == "export"
    categories = [] if status == "notFound" else [
        {"category": "records", "action": "exported" if exported else "erased", "count": 3}
    ]
    restrictions = []
    if status == "restricted":
        categories.append({"category": "invoices", "action": "retained", "count": 1})
        restrictions = [{"category": "invoices", "reason": "Statutory retention",
                         "legalBasis": "Fixture retention obligation", "retainUntil": "2030-12-31"}]
    parts = []
    if exported:
        for number, records in enumerate([[{"value": 1}, {"value": 2}], [{"value": 3}]], 1):
            body = compact({"category": "records", "records": records})
            (ROOT / f"export-part-{number}.json").write_bytes(body)
            parts.append({"partNumber": number, "sha256": hashlib.sha256(body).hexdigest(),
                          "bytes": len(body)})
    payload = {
        "receiptType": "baukit.data-subject.receipt.v1",
        "contractVersion": "1",
        "operationId": f"00000000-0000-4000-8000-{index:012d}",
        "kind": "export" if exported else "erasure",
        "status": "completed" if exported else status,
        "subjectReference": "fixture-reference-01",
        "service": service,
        "createdAt": "2026-10-20T10:00:00Z",
        "completedAt": "2026-10-20T10:00:01Z",
        "categories": categories,
        "restrictions": restrictions,
        "parts": parts,
    }
    signed = envelope(status, compact(payload))
    cases.append({"file": f"{status}.envelope.json", "result": "accepted", "canonical": True})
    if status == "completed":
        completed = signed
        # Different field order and whitespace prove verification uses received bytes.
        reordered = dict(reversed(list(payload.items())))
        envelope("openssl-noncanonical", json.dumps(reordered, indent=2).encode() + b"\n")
        cases.append({"file": "openssl-noncanonical.envelope.json", "result": "accepted",
                      "canonical": False})

tampered = json.loads(json.dumps(completed))
body = base64.b64decode(tampered["payload"]).replace(b'"count":3', b'"count":4')
tampered["payload"] = base64.b64encode(body).decode()
write_json("tampered-payload.envelope.json", tampered)
wrong_type = json.loads(json.dumps(completed))
wrong_type["payloadType"] = "application/json"
write_json("wrong-payload-type.envelope.json", wrong_type)
unknown = json.loads(json.dumps(completed))
unknown["signatures"][0]["keyid"] = "0" * 64
write_json("unknown-key.envelope.json", unknown)
for name, result in [("tampered-payload", "signature"), ("wrong-payload-type", "payloadType"),
                     ("unknown-key", "unknownKey")]:
    cases.append({"file": f"{name}.envelope.json", "result": result, "canonical": False})
write_json("cases.json", cases)
print(f"Generated {len(cases)} test-only DSSE cases using OpenSSL")
