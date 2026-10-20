use std::collections::BTreeMap;

use aws_lc_rs::{
    digest::{SHA256, digest},
    encoding::AsDer,
    signature::{ED25519, Ed25519KeyPair, KeyPair, ParsedPublicKey},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use zeroize::Zeroizing;

use crate::{
    CONTRACT_VERSION, DiscoveryDocument, DsseEnvelope, DsseSignature, ReceiptKey,
    ReceiptKeyAlgorithm, ReceiptKeyStatus, ReceiptPayload, validate_subject_reference,
};

/// DSSE payload media type for v1 receipts.
pub const RECEIPT_PAYLOAD_TYPE: &str = "application/vnd.baukit.data-subject.receipt.v1+json";
/// Receipt body's type marker.
pub const RECEIPT_TYPE: &str = "baukit.data-subject.receipt.v1";

/// Safe receipt failure classes, without key material or untrusted payload text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    /// Private key is not an unencrypted Ed25519 PKCS#8 key.
    #[error("invalid data subject signing key")]
    SigningKey,
    /// Discovery has invalid keys, duplicate IDs, or not exactly one current key.
    #[error("invalid data subject receipt keys")]
    ReceiptKeys,
    /// Discovery or receipt declares an unsupported contract version.
    #[error("unsupported data subject contract version")]
    ContractVersion,
    /// DSSE envelope declares an unexpected media type.
    #[error("invalid data subject receipt payload type")]
    PayloadType,
    /// Body or signature is not standard padded base64.
    #[error("invalid data subject receipt encoding")]
    Encoding,
    /// Envelope references a key absent from the trusted discovery keys.
    #[error("unknown data subject receipt key")]
    UnknownKey,
    /// Envelope has no valid trusted signature.
    #[error("invalid data subject receipt signature")]
    Signature,
    /// Signed JSON is invalid or has invalid receipt markers or state.
    #[error("invalid data subject receipt payload")]
    Payload,
}

/// DSSE v1 pre-authentication encoding, with decimal byte lengths and ASCII spaces.
/// The payload bytes are appended unchanged, including any whitespace.
pub fn pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "DSSEv1 {} {} {} ",
        payload_type.len(),
        payload_type,
        payload.len()
    )
    .into_bytes();
    bytes.extend_from_slice(payload);
    bytes
}

fn key_id(spki_der: &[u8]) -> String {
    digest(&SHA256, spki_der)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl ReceiptPayload {
    fn validate(&self) -> Result<(), ReceiptError> {
        if self.contract_version != CONTRACT_VERSION {
            return Err(ReceiptError::ContractVersion);
        }
        if self.receipt_type != RECEIPT_TYPE
            || !self.status.has_receipt()
            || self.completed_at < self.created_at
            || validate_subject_reference(&self.subject_reference).is_err()
        {
            return Err(ReceiptError::Payload);
        }
        Ok(())
    }

    /// Stable UTF-8 JSON for signing: struct field order, compact encoding, no trailing newline.
    /// Nested structs keep declaration order; arrays keep caller order. No map sorting occurs.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ReceiptError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| ReceiptError::Payload)
    }
}

/// Ed25519 signer. Private key material has no `Debug` or serialization implementation.
pub struct ReceiptSigner {
    pair: Ed25519KeyPair,
    key: ReceiptKey,
}

impl ReceiptSigner {
    /// Loads an unencrypted PKCS#8 PEM with the `PRIVATE KEY` label.
    /// The caller owns and must protect or erase its input buffer.
    pub fn from_pkcs8_pem(private_key_pem: &str) -> Result<Self, ReceiptError> {
        let pem = pem::parse(private_key_pem).map_err(|_| ReceiptError::SigningKey)?;
        if pem.tag() != "PRIVATE KEY" {
            return Err(ReceiptError::SigningKey);
        }
        let der = Zeroizing::new(pem.into_contents());
        Self::from_pkcs8_der(&der)
    }

    /// Loads an unencrypted PKCS#8 DER key, accepting v1 and v2 documents.
    pub fn from_pkcs8_der(private_key_der: &[u8]) -> Result<Self, ReceiptError> {
        let pair =
            Ed25519KeyPair::from_pkcs8(private_key_der).map_err(|_| ReceiptError::SigningKey)?;
        let spki = pair
            .public_key()
            .as_der()
            .map_err(|_| ReceiptError::SigningKey)?;
        let public_key_pem = pem::encode_config(
            &pem::Pem::new("PUBLIC KEY", spki.as_ref()),
            pem::EncodeConfig::new().set_line_ending(pem::LineEnding::LF),
        );
        let key = ReceiptKey {
            key_id: key_id(spki.as_ref()),
            algorithm: ReceiptKeyAlgorithm::Ed25519,
            public_key_pem,
            status: ReceiptKeyStatus::Current,
        };
        Ok(Self { pair, key })
    }

    /// Current public key for discovery, including its SPKI-derived ID.
    pub fn receipt_key(&self) -> &ReceiptKey {
        &self.key
    }

    /// Signs the canonical receipt JSON with one standard-base64 Ed25519 signature.
    pub fn sign(&self, payload: &ReceiptPayload) -> Result<DsseEnvelope, ReceiptError> {
        let body = payload.canonical_bytes()?;
        let signature = self.pair.sign(&pae(RECEIPT_PAYLOAD_TYPE, &body));
        Ok(DsseEnvelope {
            payload_type: RECEIPT_PAYLOAD_TYPE.into(),
            payload: STANDARD.encode(body),
            signatures: vec![DsseSignature {
                keyid: self.key.key_id.clone(),
                sig: STANDARD.encode(signature.as_ref()),
            }],
        })
    }
}

/// Verifies receipts against keys obtained from a trusted service's discovery document.
/// Discovery transport and the expected service identity remain the caller's responsibility.
pub struct ReceiptVerifier {
    keys: BTreeMap<String, ParsedPublicKey>,
}

impl ReceiptVerifier {
    /// Loads trusted discovery, checking the version and every advertised key.
    pub fn from_discovery(discovery: &DiscoveryDocument) -> Result<Self, ReceiptError> {
        if discovery.contract_version != CONTRACT_VERSION {
            return Err(ReceiptError::ContractVersion);
        }
        Self::from_receipt_keys(&discovery.receipt_keys)
    }

    /// Loads discovery's `receiptKeys`, requiring unique SPKI-derived IDs and one current key.
    pub fn from_receipt_keys(receipt_keys: &[ReceiptKey]) -> Result<Self, ReceiptError> {
        if receipt_keys
            .iter()
            .filter(|key| key.status == ReceiptKeyStatus::Current)
            .count()
            != 1
        {
            return Err(ReceiptError::ReceiptKeys);
        }
        let mut keys = BTreeMap::new();
        for key in receipt_keys {
            let pem = pem::parse(&key.public_key_pem).map_err(|_| ReceiptError::ReceiptKeys)?;
            let der = pem.contents();
            // RFC 8410 Ed25519 SPKI has absent parameters and a 32-byte public key.
            const SPKI_PREFIX: &[u8] = &[0x30, 0x2a, 0x30, 5, 6, 3, 0x2b, 0x65, 0x70, 3, 0x21, 0];
            if pem.tag() != "PUBLIC KEY"
                || der.len() != SPKI_PREFIX.len() + 32
                || !der.starts_with(SPKI_PREFIX)
                || key_id(der) != key.key_id
            {
                return Err(ReceiptError::ReceiptKeys);
            }
            let parsed =
                ParsedPublicKey::new(&ED25519, der).map_err(|_| ReceiptError::ReceiptKeys)?;
            if keys.insert(key.key_id.clone(), parsed).is_some() {
                return Err(ReceiptError::ReceiptKeys);
            }
        }
        Ok(Self { keys })
    }

    /// Checks the media type, all key IDs, and at least one signature before parsing JSON.
    /// Signatures cover decoded payload bytes as received, never re-serialized JSON.
    pub fn verify(&self, envelope: &DsseEnvelope) -> Result<ReceiptPayload, ReceiptError> {
        if envelope.payload_type != RECEIPT_PAYLOAD_TYPE {
            return Err(ReceiptError::PayloadType);
        }
        let payload = STANDARD
            .decode(&envelope.payload)
            .map_err(|_| ReceiptError::Encoding)?;
        let input = pae(&envelope.payload_type, &payload);
        let mut valid = false;
        for signature in &envelope.signatures {
            let key = self
                .keys
                .get(&signature.keyid)
                .ok_or(ReceiptError::UnknownKey)?;
            let sig = STANDARD
                .decode(&signature.sig)
                .map_err(|_| ReceiptError::Encoding)?;
            valid |= key.verify_sig(&input, &sig).is_ok();
        }
        if !valid {
            return Err(ReceiptError::Signature);
        }
        let parsed: ReceiptPayload =
            serde_json::from_slice(&payload).map_err(|_| ReceiptError::Payload)?;
        parsed.validate()?;
        Ok(parsed)
    }
}
