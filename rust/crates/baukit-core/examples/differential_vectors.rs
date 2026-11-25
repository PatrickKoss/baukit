//! Writes the D-06 (CSV) and D-07 (limits) differential vectors that the
//! TypeScript tests replay. Run from the repository root:
//! `cargo run -p baukit-core --example differential_vectors`.

use std::{error::Error, fs, path::Path};

use baukit_core::{
    export::{CsvCell, CsvOptions, encode_csv},
    limits::{compact_json_utf8_bytes, trimmed_unicode_scalar_count},
};
use serde_json::{Value, json};

const SEED: u64 = 0x005e_edd0_6d07;
const RANDOM_CASES: usize = 200;
const MAX_CELLS: u64 = 4;
const MAX_UNITS: u64 = 6;

const FIXED_NUMBERS: [f64; 9] = [
    1e21,
    1e-7,
    -0.0,
    0.0,
    1.5,
    -1.5e-7,
    1.25e22,
    123_456.789,
    5e-324,
];
const ALPHABET: [u16; 20] = [
    0x3d, 0x2b, 0x2d, 0x40, 0xff1d, 0xff0b, 0xff0d, 0xff20, 0x20, 0x09, 0x0d, 0x0a, 0x22, 0x2c,
    0x61, 0x3000, 0x2028, 0x00a0, 0xd83d, 0xde00,
];

/// Deterministic xorshift so the vectors never change for a fixed seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick<T: Copy>(&mut self, values: &[T]) -> Option<T> {
        let index = usize::try_from(self.below(u64::try_from(values.len()).ok()?)).ok()?;
        values.get(index).copied()
    }

    fn units(&mut self) -> Vec<u16> {
        (0..self.below(MAX_UNITS + 1))
            .filter_map(|_| self.pick(&ALPHABET))
            .collect()
    }
}

fn csv_case(cells: &[Value]) -> Result<Value, Box<dyn Error>> {
    let parsed = cells
        .iter()
        .map(|cell| match cell {
            Value::String(text) => Ok(CsvCell::text(text.clone())),
            Value::Number(number) => number
                .as_f64()
                .map(CsvCell::from)
                .ok_or_else(|| "number out of range".into()),
            _ => Err::<CsvCell<'_>, Box<dyn Error>>("unsupported cell".into()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let encoded = encode_csv([parsed], CsvOptions::new()).map_or_else(
        |error| json!({ "error": error.code() }),
        |text| json!({ "csv": text }),
    );
    Ok(json!({ "cells": cells, "expected": encoded }))
}

fn csv_vectors(rng: &mut Rng) -> Result<Value, Box<dyn Error>> {
    let mut cases = FIXED_NUMBERS
        .iter()
        .map(|number| csv_case(&[json!(number)]))
        .collect::<Result<Vec<_>, _>>()?;
    for _ in 0..RANDOM_CASES {
        let cells = (0..=rng.below(MAX_CELLS))
            .map(|_| {
                let units = rng.units();
                String::from_utf16(&units).map_or_else(|_| json!("a"), Value::String)
            })
            .collect::<Vec<_>>();
        cases.push(csv_case(&cells)?);
    }
    Ok(json!({ "seed": SEED, "cases": cases }))
}

fn limits_case(units: &[u16]) -> Result<Value, Box<dyn Error>> {
    let expected = match String::from_utf16(units) {
        Ok(text) => json!({
            "trimmedUnicodeScalarCount": trimmed_unicode_scalar_count(&text),
            "compactJsonUtf8Bytes": compact_json_utf8_bytes(&json!({ "value": text, "list": [text, 1, -2] }))?,
        }),
        Err(_) => json!({ "error": "invalid_unicode" }),
    };
    Ok(json!({ "utf16": units, "expected": expected }))
}

fn limits_vectors(rng: &mut Rng) -> Result<Value, Box<dyn Error>> {
    let fixed: [&[u16]; 5] = [
        &[0xd800],
        &[0xdc00],
        &[0x61, 0xd83d],
        &[0xde00, 0xd83d],
        &[],
    ];
    let mut cases = fixed
        .iter()
        .map(|units| limits_case(units))
        .collect::<Result<Vec<_>, _>>()?;
    for _ in 0..RANDOM_CASES {
        cases.push(limits_case(&rng.units())?);
    }
    Ok(json!({ "seed": SEED, "cases": cases }))
}

fn write(path: &Path, value: &Value) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures");
    let mut rng = Rng(SEED);
    write(
        &fixtures.join("export-csv/generated-v1.json"),
        &csv_vectors(&mut rng)?,
    )?;
    write(
        &fixtures.join("limits/generated-v1.json"),
        &limits_vectors(&mut rng)?,
    )?;
    Ok(())
}
