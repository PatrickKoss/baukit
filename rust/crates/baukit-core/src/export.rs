//! RFC 4180 CSV encoding with spreadsheet formula neutralization.

use std::borrow::{Borrow, Cow};

use thiserror::Error;

const BYTE_ORDER_MARK: char = '\u{feff}';
const RECORD_SEPARATOR: &str = "\r\n";
const FORMULA_ESCAPE: char = '\'';
const FORMULA_TRIGGERS: [char; 4] = ['=', '+', '-', '@'];
const CONTROL_TRIGGERS: [char; 2] = ['\t', '\r'];
const LEADING_WHITESPACE: [char; 4] = [' ', '\t', '\r', '\n'];
const QUOTE_REQUIRED: [char; 4] = ['"', ',', '\r', '\n'];

/// One CSV cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsvCell<'a> {
    /// An empty cell.
    Empty,
    /// Text that is neutralized when it starts like a spreadsheet formula.
    Text(Cow<'a, str>),
    /// A JSON-grammar number, written unchanged and never neutralized.
    Numeric(Cow<'a, str>),
}

impl<'a> CsvCell<'a> {
    /// Creates a text cell.
    #[must_use]
    pub fn text(value: impl Into<Cow<'a, str>>) -> Self {
        Self::Text(value.into())
    }

    /// Creates a numeric cell from preformatted text such as `-5` or `1.50`.
    #[must_use]
    pub fn numeric(value: impl Into<Cow<'a, str>>) -> Self {
        Self::Numeric(value.into())
    }
}

impl<'a> From<&'a str> for CsvCell<'a> {
    fn from(value: &'a str) -> Self {
        Self::text(value)
    }
}

impl From<String> for CsvCell<'_> {
    fn from(value: String) -> Self {
        Self::text(value)
    }
}

impl From<i64> for CsvCell<'_> {
    fn from(value: i64) -> Self {
        Self::numeric(value.to_string())
    }
}

impl From<u64> for CsvCell<'_> {
    fn from(value: u64) -> Self {
        Self::numeric(value.to_string())
    }
}

impl From<f64> for CsvCell<'_> {
    fn from(value: f64) -> Self {
        Self::numeric(value.to_string())
    }
}

impl<'a, T: Into<CsvCell<'a>>> From<Option<T>> for CsvCell<'a> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Empty, Into::into)
    }
}

/// Options for [`encode_csv`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CsvOptions {
    neutralize_formulas: bool,
    byte_order_mark: bool,
}

impl CsvOptions {
    /// Neutralizes formulas and writes no byte-order mark.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            neutralize_formulas: true,
            byte_order_mark: false,
        }
    }

    /// Writes text cells unchanged even when they start like a formula.
    #[must_use]
    pub const fn without_formula_neutralization(mut self) -> Self {
        self.neutralize_formulas = false;
        self
    }

    /// Starts the output with a UTF-8 byte-order mark.
    #[must_use]
    pub const fn with_byte_order_mark(mut self) -> Self {
        self.byte_order_mark = true;
        self
    }
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// A cell the encoder cannot write.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum CsvEncodeError {
    /// A numeric cell does not match the JSON number grammar.
    #[error("invalid numeric CSV cell at row {row_index}, column {column_index}")]
    InvalidNumericCell {
        /// Zero-based row index.
        row_index: usize,
        /// Zero-based column index.
        column_index: usize,
    },
}

impl CsvEncodeError {
    /// Returns the stable error code shared with `@baukit/data-contracts`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidNumericCell { .. } => "invalid_numeric_cell",
        }
    }
}

/// Encodes rows as RFC 4180 CSV with CRLF record separators.
///
/// # Errors
///
/// Returns [`CsvEncodeError::InvalidNumericCell`] when a numeric cell is not a JSON-grammar number.
pub fn encode_csv<'a, Rows, Row, Cell>(
    rows: Rows,
    options: CsvOptions,
) -> Result<String, CsvEncodeError>
where
    Rows: IntoIterator<Item = Row>,
    Row: IntoIterator<Item = Cell>,
    Cell: Borrow<CsvCell<'a>>,
{
    let mut output = String::new();
    if options.byte_order_mark {
        output.push(BYTE_ORDER_MARK);
    }
    for (row_index, row) in rows.into_iter().enumerate() {
        push_record(&mut output, row, row_index, options)?;
        output.push_str(RECORD_SEPARATOR);
    }
    Ok(output)
}

fn push_record<'a, Row, Cell>(
    output: &mut String,
    row: Row,
    row_index: usize,
    options: CsvOptions,
) -> Result<(), CsvEncodeError>
where
    Row: IntoIterator<Item = Cell>,
    Cell: Borrow<CsvCell<'a>>,
{
    let mut cells = row.into_iter().enumerate().peekable();
    let Some((_, first)) = cells.next() else {
        return Ok(());
    };
    if cells.peek().is_none() && is_empty(first.borrow()) {
        output.push_str("\"\"");
        return Ok(());
    }
    push_cell(output, first.borrow(), row_index, 0, options)?;
    for (column_index, cell) in cells {
        output.push(',');
        push_cell(output, cell.borrow(), row_index, column_index, options)?;
    }
    Ok(())
}

fn is_empty(cell: &CsvCell<'_>) -> bool {
    match cell {
        CsvCell::Empty => true,
        CsvCell::Text(text) => text.is_empty(),
        CsvCell::Numeric(_) => false,
    }
}

fn push_cell(
    output: &mut String,
    cell: &CsvCell<'_>,
    row_index: usize,
    column_index: usize,
    options: CsvOptions,
) -> Result<(), CsvEncodeError> {
    match cell {
        CsvCell::Empty => {}
        CsvCell::Numeric(value) => {
            if !is_json_number(value) {
                return Err(CsvEncodeError::InvalidNumericCell {
                    row_index,
                    column_index,
                });
            }
            output.push_str(value);
        }
        CsvCell::Text(value) => push_text(output, value, options),
    }
    Ok(())
}

fn push_text(output: &mut String, value: &str, options: CsvOptions) {
    let escape = options.neutralize_formulas && starts_like_formula(value);
    if !value.contains(QUOTE_REQUIRED) {
        if escape {
            output.push(FORMULA_ESCAPE);
        }
        output.push_str(value);
        return;
    }
    output.push('"');
    if escape {
        output.push(FORMULA_ESCAPE);
    }
    output.push_str(&value.replace('"', "\"\""));
    output.push('"');
}

fn starts_like_formula(value: &str) -> bool {
    if value.starts_with(CONTROL_TRIGGERS) {
        return true;
    }
    value
        .trim_start_matches(LEADING_WHITESPACE)
        .starts_with(FORMULA_TRIGGERS)
}

fn is_json_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'-'));
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => index = skip_digits(bytes, index),
        _ => return false,
    }
    if bytes.get(index) == Some(&b'.') {
        index = match required_digits(bytes, index + 1) {
            Some(end) => end,
            None => return false,
        };
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        index = match required_digits(bytes, index) {
            Some(end) => end,
            None => return false,
        };
    }
    index == bytes.len()
}

fn skip_digits(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .map_or(bytes.len(), |offset| start + offset)
}

fn required_digits(bytes: &[u8], start: usize) -> Option<usize> {
    let end = skip_digits(bytes, start.min(bytes.len()));
    (end > start).then_some(end)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureCorpus {
        version: u32,
        byte_order_mark_code_point: u32,
        cases: Vec<EncodeFixture>,
        rejections: Vec<RejectFixture>,
    }

    #[derive(Debug, Deserialize)]
    struct EncodeFixture {
        name: String,
        #[serde(default)]
        options: FixtureOptions,
        rows: Vec<Vec<FixtureCell>>,
        csv: String,
    }

    #[derive(Debug, Deserialize)]
    struct RejectFixture {
        name: String,
        #[serde(default)]
        options: FixtureOptions,
        rows: Vec<Vec<FixtureCell>>,
        error: FixtureError,
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureOptions {
        neutralize_formulas: Option<bool>,
        byte_order_mark: Option<bool>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum FixtureCell {
        Text(String),
        Numeric { numeric: String },
        Empty,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureError {
        code: String,
        row_index: usize,
        column_index: usize,
    }

    fn fixtures() -> FixtureCorpus {
        serde_json::from_str(include_str!(
            "../../../../fixtures/export-csv/csv-encoding-v1.json"
        ))
        .expect("CSV fixture should parse")
    }

    fn options(fixture: &FixtureOptions) -> CsvOptions {
        let mut options = CsvOptions::new();
        if fixture.neutralize_formulas == Some(false) {
            options = options.without_formula_neutralization();
        }
        if fixture.byte_order_mark == Some(true) {
            options = options.with_byte_order_mark();
        }
        options
    }

    fn rows(fixture: &[Vec<FixtureCell>]) -> Vec<Vec<CsvCell<'_>>> {
        fixture
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        FixtureCell::Text(text) => CsvCell::text(text.as_str()),
                        FixtureCell::Numeric { numeric } => CsvCell::numeric(numeric.as_str()),
                        FixtureCell::Empty => CsvCell::Empty,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn shared_fixture_encodes_every_case() {
        let corpus = fixtures();
        assert_eq!(corpus.version, 1);
        assert!(!corpus.cases.is_empty());
        let byte_order_mark = char::from_u32(corpus.byte_order_mark_code_point)
            .expect("fixture byte-order mark should be a scalar value");
        assert_eq!(byte_order_mark, BYTE_ORDER_MARK);
        for case in corpus.cases {
            let encoded = encode_csv(rows(&case.rows), options(&case.options))
                .unwrap_or_else(|error| panic!("{}: {error}", case.name));
            let expected = if case.options.byte_order_mark == Some(true) {
                format!("{byte_order_mark}{}", case.csv)
            } else {
                case.csv
            };
            assert_eq!(encoded, expected, "{}", case.name);
        }
    }

    #[test]
    fn byte_order_mark_is_the_first_code_point_only_when_requested() {
        let rows = [["a"]];
        let marked = encode_csv(
            rows.map(|row| row.map(CsvCell::from)),
            CsvOptions::new().with_byte_order_mark(),
        )
        .expect("text encodes");
        assert_eq!(marked.chars().next().map(u32::from), Some(0xfeff));
        let plain = encode_csv(rows.map(|row| row.map(CsvCell::from)), CsvOptions::new())
            .expect("text encodes");
        assert_eq!(plain, "a\r\n");
    }

    #[test]
    fn shared_fixture_rejects_every_invalid_numeric_cell() {
        let corpus = fixtures();
        assert!(!corpus.rejections.is_empty());
        for case in corpus.rejections {
            let error = encode_csv(rows(&case.rows), options(&case.options)).expect_err(&case.name);
            assert_eq!(error.code(), case.error.code, "{}", case.name);
            assert_eq!(
                error,
                CsvEncodeError::InvalidNumericCell {
                    row_index: case.error.row_index,
                    column_index: case.error.column_index,
                },
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn conversions_pick_text_numeric_and_empty_cells() {
        let rows: [Vec<CsvCell<'_>>; 1] = [vec![
            CsvCell::from("-5"),
            CsvCell::from(String::from("=x")),
            CsvCell::from(-5_i64),
            CsvCell::from(7_u64),
            CsvCell::from(-0.25_f64),
            CsvCell::from(None::<i64>),
            CsvCell::from(Some("ok")),
        ]];
        assert_eq!(
            encode_csv(&rows, CsvOptions::default()),
            Ok(String::from("'-5,'=x,-5,7,-0.25,,ok\r\n"))
        );
    }

    #[test]
    fn non_finite_floats_are_rejected() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let rows = [[CsvCell::from(value)]];
            assert_eq!(
                encode_csv(&rows, CsvOptions::new()),
                Err(CsvEncodeError::InvalidNumericCell {
                    row_index: 0,
                    column_index: 0,
                })
            );
        }
    }

    #[test]
    fn error_messages_do_not_echo_cell_values() {
        let rows = [[CsvCell::numeric("=secret()")]];
        let error = encode_csv(&rows, CsvOptions::new()).expect_err("formula is not numeric");
        assert!(!error.to_string().contains("secret"));
    }
}
