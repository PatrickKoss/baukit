use aws_lc_rs::digest::{SHA256, digest};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_data_subject::{
    CONTRACT_VERSION, DsseEnvelope, ExportPart, OperationError, OperationResource, OperationStatus,
    RECEIPT_TYPE, ReceiptPayload,
};

use super::{DataSubjectFault, DataSubjectOutcome, Operation, TargetState, lock};

fn apply_outcome(state: &TargetState, operation: &mut Operation) {
    let resource = &mut operation.resource;
    resource.completed_at = Some(state.now());
    match &operation.outcome {
        DataSubjectOutcome::Completed { categories } => {
            resource.status = OperationStatus::Completed;
            resource.categories = categories.clone();
        }
        DataSubjectOutcome::NotFound => resource.status = OperationStatus::NotFound,
        DataSubjectOutcome::Restricted {
            categories,
            restrictions,
        } => {
            resource.status = OperationStatus::Restricted;
            resource.categories = categories.clone();
            resource.restrictions = restrictions.clone();
        }
        DataSubjectOutcome::Failed { code } => {
            resource.status = OperationStatus::Failed;
            resource.error = Some(OperationError { code: code.clone() });
        }
        DataSubjectOutcome::Export { parts } => {
            resource.status = OperationStatus::Completed;
            operation.parts = parts
                .iter()
                .map(|part| serde_json::to_vec(part).expect("JSON value serializes"))
                .collect();
            resource.parts = operation
                .parts
                .iter()
                .enumerate()
                .map(|(index, bytes)| ExportPart {
                    part_number: u32::try_from(index + 1).expect("validated part count"),
                    sha256: hex_digest(bytes),
                    bytes: bytes.len() as u64,
                })
                .collect();
        }
        DataSubjectOutcome::PendingFor { .. } => {
            unreachable!("pending wrappers consumed before completion")
        }
    }
}

pub(super) fn complete(state: &TargetState, operation: &mut Operation) {
    apply_outcome(state, operation);
    if !operation.resource.status.has_receipt() {
        return;
    }
    let faults = lock(&operation.script).faults.clone();
    let receipt = signed_receipt(state, &operation.resource, &faults);
    if has_fault(&faults, |fault| {
        matches!(fault, DataSubjectFault::PartDigestMismatch)
    }) {
        corrupt_parts(&mut operation.parts);
    }
    operation.resource.receipt = Some(receipt);
}

fn has_fault(faults: &[DataSubjectFault], is_fault: fn(&DataSubjectFault) -> bool) -> bool {
    faults.iter().any(is_fault)
}

fn signed_receipt(
    state: &TargetState,
    resource: &OperationResource,
    faults: &[DataSubjectFault],
) -> DsseEnvelope {
    let mut payload = ReceiptPayload {
        receipt_type: RECEIPT_TYPE.into(),
        contract_version: CONTRACT_VERSION.into(),
        operation_id: resource.operation_id,
        kind: resource.kind,
        status: resource.status,
        subject_reference: resource.subject_reference.clone(),
        service: state.discovery.service.clone(),
        created_at: resource.created_at,
        completed_at: resource.completed_at.expect("terminal timestamp"),
        categories: resource.categories.clone(),
        restrictions: resource.restrictions.clone(),
        parts: resource.parts.clone(),
    };
    if has_fault(faults, |fault| {
        matches!(fault, DataSubjectFault::WrongSubjectReference)
    }) {
        flip_last_reference_char(&mut payload.subject_reference);
    }
    let signer = if has_fault(faults, |fault| {
        matches!(fault, DataSubjectFault::BadSignature)
    }) {
        &state.unadvertised_signer
    } else {
        &state.signer
    };
    let mut receipt = signer.sign(&payload).expect("controlled receipt is valid");
    if has_fault(faults, |fault| {
        matches!(fault, DataSubjectFault::WrongKeyId)
    }) {
        receipt.signatures[0].keyid = "0".repeat(64);
    }
    if has_fault(faults, |fault| {
        matches!(fault, DataSubjectFault::TamperedPayload)
    }) {
        let mut bytes = STANDARD.decode(&receipt.payload).expect("signer base64");
        bytes.push(b' ');
        receipt.payload = STANDARD.encode(bytes);
    }
    receipt
}

fn flip_last_reference_char(reference: &mut String) {
    let last = reference.pop().expect("validated reference");
    reference.push(if last == 'x' { 'y' } else { 'x' });
}

fn corrupt_parts(parts: &mut [Vec<u8>]) {
    for bytes in parts {
        let mut changed = vec![b' '; bytes.len()];
        changed[0] = if bytes == b"0" { b'1' } else { b'0' };
        *bytes = changed;
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
