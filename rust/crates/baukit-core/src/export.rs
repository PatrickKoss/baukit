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
    quote_all_cells: bool,
    null_marker: Option<&'static str>,
}

impl CsvOptions {
    /// Neutralizes formulas and writes no byte-order mark.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            neutralize_formulas: true,
            byte_order_mark: false,
            quote_all_cells: false,
            null_marker: None,
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

    /// Quotes every text and numeric cell. Empty cells stay unquoted, so a reader can tell them
    /// from empty text.
    #[must_use]
    pub const fn with_all_cells_quoted(mut self) -> Self {
        self.quote_all_cells = true;
        self
    }

    /// Writes `marker` unquoted for an empty cell and quotes a text cell with the same content.
    ///
    /// # Panics
    ///
    /// Panics when `marker` is empty or contains a double quote, comma, CR, or LF. In a `const`
    /// item the check fails the build instead.
    #[must_use]
    pub const fn with_null_marker(mut self, marker: &'static str) -> Self {
        assert!(
            is_valid_null_marker(marker),
            "CSV null marker must be non-empty and free of double quotes, commas, CR, and LF"
        );
        self.null_marker = Some(marker);
        self
    }
}

const fn is_valid_null_marker(marker: &str) -> bool {
    let bytes = marker.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        if matches!(bytes[index], b'"' | b',' | b'\r' | b'\n') {
            return false;
        }
        index += 1;
    }
    true
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
    let record_start = output.len();
    let mut column_count = 0;
    for (column_index, cell) in row.into_iter().enumerate() {
        if column_index > 0 {
            output.push(',');
        }
        push_cell(output, cell.borrow(), row_index, column_index, options)?;
        column_count += 1;
    }
    if column_count == 1 && output.len() == record_start {
        output.push_str("\"\"");
    }
    Ok(())
}

fn push_cell(
    output: &mut String,
    cell: &CsvCell<'_>,
    row_index: usize,
    column_index: usize,
    options: CsvOptions,
) -> Result<(), CsvEncodeError> {
    match cell {
        CsvCell::Empty => output.push_str(options.null_marker.unwrap_or_default()),
        CsvCell::Numeric(value) => {
            if !is_json_number(value) {
                return Err(CsvEncodeError::InvalidNumericCell {
                    row_index,
                    column_index,
                });
            }
            push_numeric(output, value, options);
        }
        CsvCell::Text(value) => push_text(output, value, options),
    }
    Ok(())
}

fn push_numeric(output: &mut String, value: &str, options: CsvOptions) {
    if !options.quote_all_cells {
        output.push_str(value);
        return;
    }
    output.push('"');
    output.push_str(value);
    output.push('"');
}

fn push_text(output: &mut String, value: &str, options: CsvOptions) {
    let escape = options.neutralize_formulas && starts_like_formula(value);
    if !must_quote(value, escape, options) {
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

fn must_quote(value: &str, escape: bool, options: CsvOptions) -> bool {
    options.quote_all_cells
        || value.contains(QUOTE_REQUIRED)
        || reads_as_null_marker(value, escape, options)
}

fn reads_as_null_marker(value: &str, escape: bool, options: CsvOptions) -> bool {
    let Some(marker) = options.null_marker else {
        return false;
    };
    let unescaped = if escape {
        marker.strip_prefix(FORMULA_ESCAPE)
    } else {
        Some(marker)
    };
    unescaped == Some(value)
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
        quote_all_cells: Option<bool>,
        null_marker: Option<String>,
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
        if fixture.quote_all_cells == Some(true) {
            options = options.with_all_cells_quoted();
        }
        if let Some(marker) = &fixture.null_marker {
            options = options.with_null_marker(String::leak(marker.clone()));
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
    fn null_marker_is_accepted_in_a_const_item() {
        const OPTIONS: CsvOptions = CsvOptions::new().with_null_marker("\\N");
        let rows = [[CsvCell::Empty, CsvCell::text("\\N")]];
        assert_eq!(
            encode_csv(&rows, OPTIONS),
            Ok(String::from("\\N,\"\\N\"\r\n"))
        );
    }

    #[test]
    fn invalid_null_markers_panic() {
        for marker in ["", "a,b", "say \"null\"", "line\n", "line\r"] {
            let result = std::panic::catch_unwind(|| CsvOptions::new().with_null_marker(marker));
            assert!(result.is_err(), "{marker:?}");
        }
    }

    #[test]
    fn a_neutralized_cell_is_quoted_when_it_would_read_as_the_null_marker() {
        let rows = [[CsvCell::Empty, CsvCell::text("-")]];
        assert_eq!(
            encode_csv(&rows, CsvOptions::new().with_null_marker("'-")),
            Ok(String::from("'-,\"'-\"\r\n"))
        );
    }

    #[test]
    fn error_messages_do_not_echo_cell_values() {
        let rows = [[CsvCell::numeric("=secret()")]];
        let error = encode_csv(&rows, CsvOptions::new()).expect_err("formula is not numeric");
        assert!(!error.to_string().contains("secret"));
    }
}
