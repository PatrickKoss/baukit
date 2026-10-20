use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::IdentifierType;

/// Public service name and release version, without deployment secrets.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// Stable service name.
    pub name: String,
    /// Product release version.
    pub version: String,
}

/// Machine scopes required for each operation kind.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Scopes {
    /// Scope for starting and reading erasures.
    pub erasure: String,
    /// Scope for starting and reading exports and their parts.
    pub export: String,
}

impl Default for Scopes {
    fn default() -> Self {
        Self {
            erasure: "data-subject.erase".into(),
            export: "data-subject.export".into(),
        }
    }
}

/// Advertised bounds. Identifier lengths count UTF-8 bytes before and after normalization.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Maximum number of identifiers in one request.
    pub max_identifiers: usize,
    /// Maximum UTF-8 byte length of one identifier value.
    pub max_identifier_length: usize,
    /// Maximum byte length of a downloadable JSON export part.
    pub max_export_part_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_identifiers: 16,
            max_identifier_length: 256,
            max_export_part_bytes: 8_388_608,
        }
    }
}

/// Supported operation kind.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    /// Erase matching product data.
    Erasure,
    /// Export matching product data.
    Export,
}

/// Public operation state.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationStatus {
    /// Accepted work has not finished.
    Pending,
    /// All requested work finished.
    Completed,
    /// No identifier matched a local subject.
    NotFound,
    /// Erasure finished except for listed legal retention obligations.
    Restricted,
    /// Work failed and has no receipt.
    Failed,
}

impl OperationStatus {
    /// Whether clients should stop polling.
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }

    /// Whether this state must carry a signed receipt.
    pub const fn has_receipt(self) -> bool {
        matches!(self, Self::Completed | Self::NotFound | Self::Restricted)
    }
}

/// Signing-key publication state. A discovery document has exactly one current key.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptKeyStatus {
    /// Used for new signatures.
    Current,
    /// Kept to verify receipts still retained.
    Previous,
}

/// Supported receipt signature algorithm.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ReceiptKeyAlgorithm {
    /// Ed25519 over DSSE v1 PAE bytes.
    #[serde(rename = "ed25519")]
    Ed25519,
}

/// Public key advertised in discovery.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptKey {
    /// Lowercase hexadecimal SHA-256 of the SPKI DER.
    pub key_id: String,
    /// Receipt signature algorithm.
    pub algorithm: ReceiptKeyAlgorithm,
    /// SPKI PEM with the `PUBLIC KEY` label.
    pub public_key_pem: String,
    /// Whether the key signs new receipts or only verifies previous ones.
    pub status: ReceiptKeyStatus,
}

/// Unauthenticated `/.well-known/baukit/data-subject` response.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryDocument {
    /// Must be `1` for this contract.
    pub contract_version: String,
    /// Service that processes requests and signs receipts.
    pub service: Service,
    /// Operations implemented by the service.
    pub operations: Vec<OperationKind>,
    /// Identifier types accepted by the service.
    pub identifier_types: Vec<IdentifierType>,
    /// Required machine scopes.
    pub scopes: Scopes,
    /// Request and export bounds.
    pub limits: Limits,
    /// Current key and previous keys for retained receipts.
    pub receipt_keys: Vec<ReceiptKey>,
}

/// One transient lookup identifier. `Debug` redacts its value.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Identifier {
    /// Advertised built-in or custom identifier type.
    #[serde(rename = "type")]
    pub identifier_type: IdentifierType,
    /// Raw value, never persist or log it.
    pub value: String,
}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identifier")
            .field("identifier_type", &self.identifier_type)
            .finish_non_exhaustive()
    }
}

/// POST body shared by erasures and exports. Identifiers remain transient.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataSubjectRequest {
    /// Client's opaque correlation reference, never a raw identifier.
    pub subject_reference: String,
    /// Lookup inputs, validated against discovery before matching.
    pub identifiers: Vec<Identifier>,
}

impl fmt::Debug for DataSubjectRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataSubjectRequest { .. }")
    }
}

/// Result for one product-defined data category.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CategoryEntry {
    /// Stable category name, without subject data.
    pub category: String,
    /// What happened to this category.
    pub action: CategoryAction,
    /// Number of affected records.
    pub count: u64,
}

/// Action recorded for a data category.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CategoryAction {
    /// Records were erased.
    Erased,
    /// Records were included in the export.
    Exported,
    /// Records remain under a stated legal obligation.
    Retained,
}

/// Legal obligation that prevented erasing a category.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Restriction {
    /// Category that remains stored.
    pub category: String,
    /// Safe explanation, without subject data.
    pub reason: String,
    /// Applicable legal obligation.
    pub legal_basis: String,
    /// Retention end date, or null if no date is known.
    pub retain_until: Option<NaiveDate>,
}

/// Digest and size of the exact downloadable JSON part bytes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportPart {
    /// Number used in the part URL.
    pub part_number: u32,
    /// Lowercase hexadecimal SHA-256 of the part bytes.
    pub sha256: String,
    /// Number of part bytes, bounded by discovery's limit.
    pub bytes: u64,
}

/// Safe terminal operation error. Internal causes are never serialized here.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationError {
    /// Stable error code chosen by the service.
    pub code: String,
}

/// GET response and idempotent POST result.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationResource {
    /// Opaque operation identifier.
    pub operation_id: Uuid,
    /// Operation whose route owns this resource.
    pub kind: OperationKind,
    /// Current state.
    pub status: OperationStatus,
    /// Echo of the client's opaque correlation reference.
    pub subject_reference: String,
    /// Acceptance timestamp.
    pub created_at: DateTime<Utc>,
    /// Terminal timestamp, null while pending.
    pub completed_at: Option<DateTime<Utc>>,
    /// Category counts and actions.
    pub categories: Vec<CategoryEntry>,
    /// Legal retention obligations.
    pub restrictions: Vec<Restriction>,
    /// Completed export part metadata.
    pub parts: Vec<ExportPart>,
    /// Signed terminal proof, absent for pending and failed operations.
    pub receipt: Option<DsseEnvelope>,
    /// Safe failure code, present only for failed operations.
    pub error: Option<OperationError>,
}

/// Signed receipt body. Field declaration order is the signer's canonical JSON order.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptPayload {
    /// Must be `baukit.data-subject.receipt.v1`.
    pub receipt_type: String,
    /// Must be `1`.
    pub contract_version: String,
    /// Operation this receipt proves.
    pub operation_id: Uuid,
    /// Erasure or export.
    pub kind: OperationKind,
    /// Completed, not found, or restricted.
    pub status: OperationStatus,
    /// Echo of the client's opaque correlation reference.
    pub subject_reference: String,
    /// Service that signed this proof.
    pub service: Service,
    /// Operation acceptance time.
    pub created_at: DateTime<Utc>,
    /// Operation completion time.
    pub completed_at: DateTime<Utc>,
    /// Category counts and actions, without identifiers or record content.
    pub categories: Vec<CategoryEntry>,
    /// Legal retention obligations.
    pub restrictions: Vec<Restriction>,
    /// Digests and sizes of completed export parts.
    pub parts: Vec<ExportPart>,
}

/// DSSE v1 envelope. Payload and signatures use standard padded base64.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DsseEnvelope {
    /// `application/vnd.baukit.data-subject.receipt.v1+json`.
    pub payload_type: String,
    /// Base64 of the exact UTF-8 receipt JSON bytes.
    pub payload: String,
    /// At least one trusted Ed25519 signature is required.
    pub signatures: Vec<DsseSignature>,
}

/// One signature over the envelope's PAE bytes.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DsseSignature {
    /// DSSE's lowercase `keyid`, equal to discovery's `keyId`.
    pub keyid: String,
    /// Standard base64 of the 64-byte Ed25519 signature.
    pub sig: String,
}
