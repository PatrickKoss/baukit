use std::collections::BTreeMap;

use aws_lc_rs::digest::{SHA256, digest};
use serde_json::Value;

use super::DataSubjectError;
use crate::{CategoryAction, CategoryEntry, ExportPart};

pub(super) struct Part {
    pub metadata: ExportPart,
    pub bytes: Vec<u8>,
}

pub(super) fn split(
    categories: BTreeMap<String, Vec<Value>>,
    limit: u64,
) -> Result<(Vec<CategoryEntry>, Vec<Part>), DataSubjectError> {
    let mut entries = Vec::new();
    let mut parts = Vec::new();
    for (name, records) in categories {
        entries.push(CategoryEntry {
            category: name.clone(),
            action: CategoryAction::Exported,
            count: records.len() as u64,
        });
        let prefix = format!(
            "{{\"category\":{},\"records\":[",
            serde_json::to_string(&name)?
        );
        if prefix.len() as u64 + 2 > limit {
            return Err(DataSubjectError::ExportRecordTooLarge);
        }
        let mut bytes = prefix.as_bytes().to_vec();
        for record in records {
            let record = serde_json::to_vec(&record)?;
            match placement(bytes.len(), prefix.len(), record.len(), limit) {
                Placement::TooLarge => return Err(DataSubjectError::ExportRecordTooLarge),
                Placement::NewPart => {
                    finish(&mut parts, bytes)?;
                    bytes = prefix.as_bytes().to_vec();
                }
                Placement::Append if bytes.len() > prefix.len() => bytes.push(b','),
                Placement::Append => {}
            }
            bytes.extend(record);
        }
        finish(&mut parts, bytes)?;
    }
    Ok((entries, parts))
}

const CLOSING_BYTES: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    Append,
    NewPart,
    TooLarge,
}

/// Where a record of `record` bytes goes, given the open part holds `current` bytes of which
/// `prefix` are its header. Each part must end at most `limit` bytes after `]}` closes it.
fn placement(current: usize, prefix: usize, record: usize, limit: u64) -> Placement {
    let widen = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
    let alone = widen(prefix)
        .saturating_add(widen(record))
        .saturating_add(CLOSING_BYTES);
    if alone > limit {
        return Placement::TooLarge;
    }
    let separator = u64::from(current > prefix);
    let appended = widen(current)
        .saturating_add(separator)
        .saturating_add(widen(record))
        .saturating_add(CLOSING_BYTES);
    if appended > limit {
        Placement::NewPart
    } else {
        Placement::Append
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// K-24: any part length at least its header, any record length and any limit, all
    /// below 2^20 bytes. No loops. Whatever the placement, the closed part stays within
    /// `limit`, and a record is too large only when it cannot fit in an empty part.
    /// Bounds: part, record and limit below 2^20 bytes, no loops.
    #[kani::proof]
    fn placement_keeps_every_part_within_the_limit() {
        const BOUND: usize = 1 << 20;
        let prefix: usize = kani::any();
        let current: usize = kani::any();
        let record: usize = kani::any();
        let limit: u64 = kani::any();
        kani::assume(prefix < BOUND && current < BOUND && record < BOUND);
        kani::assume(current >= prefix && limit < BOUND as u64);
        let closed = |len: usize| len as u64 + CLOSING_BYTES;
        match placement(current, prefix, record, limit) {
            Placement::TooLarge => assert!(closed(prefix + record) > limit),
            Placement::NewPart => {
                assert!(closed(prefix + record) <= limit);
                assert!(current > prefix);
            }
            Placement::Append => {
                assert!(closed(current + usize::from(current > prefix) + record) <= limit);
            }
        }
    }
}

fn finish(parts: &mut Vec<Part>, mut bytes: Vec<u8>) -> Result<(), DataSubjectError> {
    bytes.extend(b"]}");
    parts.push(Part {
        metadata: ExportPart {
            part_number: u32::try_from(parts.len() + 1).map_err(|_| DataSubjectError::Result)?,
            sha256: digest(&SHA256, &bytes)
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            bytes: bytes.len() as u64,
        },
        bytes,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_byte_limit_counts_wrapper_escaping_unicode_and_empty_categories() {
        let document = json!({"category":"ü\"", "records":[{"value":"雪\n"}]});
        let expected = serde_json::to_vec(&document).expect("JSON");
        let categories = BTreeMap::from([("ü\"".into(), vec![json!({"value":"雪\n"})])]);
        let (_, parts) = split(categories.clone(), expected.len() as u64).expect("exact fit");
        assert_eq!(parts[0].bytes, expected);
        assert!(matches!(
            split(categories, expected.len() as u64 - 1),
            Err(DataSubjectError::ExportRecordTooLarge)
        ));
        let categories = BTreeMap::from([("empty".into(), vec![])]);
        let (_, parts) = split(categories.clone(), 33).expect("empty category");
        assert_eq!(parts[0].bytes, br#"{"category":"empty","records":[]}"#);
        assert!(matches!(
            split(categories, 1),
            Err(DataSubjectError::ExportRecordTooLarge)
        ));
    }
}
