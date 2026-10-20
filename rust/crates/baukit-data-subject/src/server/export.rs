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
            if (prefix.len() + record.len() + 2) as u64 > limit {
                return Err(DataSubjectError::ExportRecordTooLarge);
            }
            let has_records = bytes.len() > prefix.len();
            if (bytes.len() + usize::from(has_records) + record.len() + 2) as u64 > limit {
                finish(&mut parts, bytes)?;
                bytes = prefix.as_bytes().to_vec();
            }
            if bytes.len() > prefix.len() {
                bytes.push(b',');
            }
            bytes.extend(record);
        }
        finish(&mut parts, bytes)?;
    }
    Ok((entries, parts))
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
