use std::{collections::BTreeSet, fmt, future::Future, pin::Pin};

pub use baukit_core::limits::{
    compact_json_utf8_bytes as compact_document_bytes,
    trimmed_unicode_scalar_count as trimmed_text_length,
};

type IngressFuture<'a, Output> = Pin<Box<dyn Future<Output = Output> + 'a>>;

/// One named write ingress used by the parity check.
pub struct NamedIngress<'a, Output> {
    name: &'a str,
    invoke: Box<dyn FnOnce() -> IngressFuture<'a, Output> + 'a>,
}

impl<'a, Output> NamedIngress<'a, Output> {
    /// Creates an ingress from its diagnostic name and an async function.
    pub fn new<Invoke, InvokeFuture>(name: &'a str, invoke: Invoke) -> Self
    where
        Invoke: FnOnce() -> InvokeFuture + 'a,
        InvokeFuture: Future<Output = Output> + 'a,
    {
        Self {
            name,
            invoke: Box::new(move || Box::pin(invoke())),
        }
    }
}

/// One named output used by the stable reason-code check.
pub struct NamedOutput<'a, Output> {
    name: &'a str,
    output: Output,
}

impl<'a, Output> NamedOutput<'a, Output> {
    /// Creates a named output.
    pub const fn new(name: &'a str, output: Output) -> Self {
        Self { name, output }
    }
}

/// Violations found while exercising a product's resource limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LimitsConformanceError {
    violations: Vec<String>,
}

impl LimitsConformanceError {
    /// Returns violations in check order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for LimitsConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "resource-limits conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LimitsConformanceError {}

/// Checks payloads built at `limit - 1`, `limit`, and `limit + 1`.
///
/// The first two payloads must pass validation and the last must fail. A limit
/// of zero cannot express all three cases and is reported as a violation.
pub async fn check_limit_boundaries<Payload, ValidationError, Build, Validate, ValidateFuture>(
    limit: usize,
    mut build_payload: Build,
    mut validate: Validate,
) -> Result<(), LimitsConformanceError>
where
    Build: FnMut(usize) -> Payload,
    Validate: FnMut(Payload) -> ValidateFuture,
    ValidateFuture: Future<Output = Result<(), ValidationError>>,
{
    let Some(below) = limit.checked_sub(1) else {
        return failure("the boundary limit must be at least 1");
    };
    let Some(above) = limit.checked_add(1) else {
        return failure("the boundary limit must be less than usize::MAX");
    };

    let cases = [
        ("limit - 1", below, true),
        ("limit", limit, true),
        ("limit + 1", above, false),
    ];
    let mut violations = Vec::new();
    for (name, size, should_pass) in cases {
        let passed = validate(build_payload(size)).await.is_ok();
        if passed != should_pass {
            let outcome = if passed { "accepted" } else { "rejected" };
            violations.push(format!("the {name} payload was {outcome}"));
        }
    }
    finish(violations)
}

/// Panics when a validator mishandles a boundary payload.
///
/// # Panics
///
/// Panics with every boundary violation.
pub async fn assert_limit_boundaries<Payload, ValidationError, Build, Validate, ValidateFuture>(
    limit: usize,
    build_payload: Build,
    validate: Validate,
) where
    Build: FnMut(usize) -> Payload,
    Validate: FnMut(Payload) -> ValidateFuture,
    ValidateFuture: Future<Output = Result<(), ValidationError>>,
{
    if let Err(error) = check_limit_boundaries(limit, build_payload, validate).await {
        panic!("{error}");
    }
}

/// Product adapter exercised by the live-row capacity checks.
///
/// Bind an adapter instance to the owner or parent whose rows share a cap.
#[allow(async_fn_in_trait)]
pub trait LiveRowLimitAdapter {
    /// Product row returned after creation.
    type Row;
    /// Product failure returned by row operations.
    type Error;

    /// Creates one distinct live row for the bound owner or parent.
    async fn create_row(&mut self, sequence: usize) -> Result<Self::Row, Self::Error>;

    /// Updates a row without increasing the number of live rows.
    async fn update_row(&mut self, row: &Self::Row) -> Result<(), Self::Error>;

    /// Soft-deletes a row so it no longer counts toward capacity.
    async fn soft_delete_row(&mut self, row: &Self::Row) -> Result<(), Self::Error>;
}

/// Fills a live-row cap, rejects one extra row, and checks that an update succeeds.
pub async fn check_update_at_capacity<Adapter>(
    adapter: &mut Adapter,
    limit: usize,
) -> Result<(), LimitsConformanceError>
where
    Adapter: LiveRowLimitAdapter,
{
    let rows = fill_to_capacity(adapter, limit).await?;
    let mut violations = reject_extra_row(adapter, limit).await;
    if let Some(row) = rows.first()
        && adapter.update_row(row).await.is_err()
    {
        violations.push("updating a live row at capacity failed".to_owned());
    }
    finish(violations)
}

/// Panics when update-at-capacity behavior is incorrect.
///
/// # Panics
///
/// Panics with every live-row capacity violation.
pub async fn assert_update_at_capacity<Adapter>(adapter: &mut Adapter, limit: usize)
where
    Adapter: LiveRowLimitAdapter,
{
    if let Err(error) = check_update_at_capacity(adapter, limit).await {
        panic!("{error}");
    }
}

/// Fills a live-row cap, soft-deletes one row, and checks that capacity is reusable.
pub async fn check_soft_delete_capacity_reuse<Adapter>(
    adapter: &mut Adapter,
    limit: usize,
) -> Result<(), LimitsConformanceError>
where
    Adapter: LiveRowLimitAdapter,
{
    let rows = fill_to_capacity(adapter, limit).await?;
    let mut violations = reject_extra_row(adapter, limit).await;
    let Some(row) = rows.first() else {
        return finish(violations);
    };
    if adapter.soft_delete_row(row).await.is_err() {
        violations.push("soft-deleting a live row at capacity failed".to_owned());
        return finish(violations);
    }
    if adapter.create_row(limit + 1).await.is_err() {
        violations.push("creating a row after soft deletion failed".to_owned());
    }
    finish(violations)
}

/// Panics when soft-deleted capacity cannot be reused.
///
/// # Panics
///
/// Panics with every live-row capacity violation.
pub async fn assert_soft_delete_capacity_reuse<Adapter>(adapter: &mut Adapter, limit: usize)
where
    Adapter: LiveRowLimitAdapter,
{
    if let Err(error) = check_soft_delete_capacity_reuse(adapter, limit).await {
        panic!("{error}");
    }
}

/// Checks that named outputs contain the expected stable reason code.
///
/// `extract_reason` should return the public reason code from the output's
/// error shape. Missing codes and duplicate or empty path names are violations.
pub fn check_reason_code_conformance<'a, Output, Outputs, ExtractReason>(
    expected_reason: &str,
    outputs: Outputs,
    extract_reason: ExtractReason,
) -> Result<(), LimitsConformanceError>
where
    Outputs: IntoIterator<Item = NamedOutput<'a, Output>>,
    ExtractReason: for<'output> Fn(&'output Output) -> Option<&'output str>,
{
    let mut violations = validate_expected_reason(expected_reason);
    let mut names = BTreeSet::new();
    let mut output_count = 0;
    for output in outputs {
        output_count += 1;
        validate_path_name(output.name, &mut names, &mut violations);
        match extract_reason(&output.output) {
            Some(actual) if actual == expected_reason => {}
            Some(actual) => violations.push(format!(
                "path {:?} returned reason code {actual:?}; expected {expected_reason:?}",
                output.name
            )),
            None => violations.push(format!(
                "path {:?} returned no reason code; expected {expected_reason:?}",
                output.name
            )),
        }
    }
    if output_count == 0 {
        violations.push("at least one named output is required".to_owned());
    }
    finish(violations)
}

/// Panics when named outputs do not carry one expected reason code.
///
/// # Panics
///
/// Panics with every reason-code violation.
#[track_caller]
pub fn assert_reason_code_conformance<'a, Output, Outputs, ExtractReason>(
    expected_reason: &str,
    outputs: Outputs,
    extract_reason: ExtractReason,
) where
    Outputs: IntoIterator<Item = NamedOutput<'a, Output>>,
    ExtractReason: for<'output> Fn(&'output Output) -> Option<&'output str>,
{
    if let Err(error) = check_reason_code_conformance(expected_reason, outputs, extract_reason) {
        panic!("{error}");
    }
}

/// Invokes each named ingress and checks for one expected stable reason code.
pub async fn check_ingress_reason_code_parity<'a, Output, Ingresses, ExtractReason>(
    expected_reason: &str,
    ingresses: Ingresses,
    extract_reason: ExtractReason,
) -> Result<(), LimitsConformanceError>
where
    Ingresses: IntoIterator<Item = NamedIngress<'a, Output>>,
    ExtractReason: for<'output> Fn(&'output Output) -> Option<&'output str>,
{
    let mut outputs = Vec::new();
    for ingress in ingresses {
        outputs.push(NamedOutput::new(ingress.name, (ingress.invoke)().await));
    }
    check_reason_code_conformance(expected_reason, outputs, extract_reason)
}

/// Panics when named ingresses do not return one expected stable reason code.
///
/// # Panics
///
/// Panics with every ingress-parity violation.
pub async fn assert_ingress_reason_code_parity<'a, Output, Ingresses, ExtractReason>(
    expected_reason: &str,
    ingresses: Ingresses,
    extract_reason: ExtractReason,
) where
    Ingresses: IntoIterator<Item = NamedIngress<'a, Output>>,
    ExtractReason: for<'output> Fn(&'output Output) -> Option<&'output str>,
{
    if let Err(error) =
        check_ingress_reason_code_parity(expected_reason, ingresses, extract_reason).await
    {
        panic!("{error}");
    }
}

async fn fill_to_capacity<Adapter>(
    adapter: &mut Adapter,
    limit: usize,
) -> Result<Vec<Adapter::Row>, LimitsConformanceError>
where
    Adapter: LiveRowLimitAdapter,
{
    if limit == 0 {
        return failure("the live-row limit must be at least 1");
    }
    if limit == usize::MAX {
        return failure("the live-row limit must be less than usize::MAX");
    }
    let mut rows = Vec::with_capacity(limit);
    for sequence in 0..limit {
        match adapter.create_row(sequence).await {
            Ok(row) => rows.push(row),
            Err(_) => {
                return failure(format!(
                    "creating live row {} of {limit} failed before capacity",
                    sequence + 1
                ));
            }
        }
    }
    Ok(rows)
}

async fn reject_extra_row<Adapter>(adapter: &mut Adapter, limit: usize) -> Vec<String>
where
    Adapter: LiveRowLimitAdapter,
{
    if adapter.create_row(limit).await.is_ok() {
        vec!["creating a row above capacity succeeded".to_owned()]
    } else {
        Vec::new()
    }
}

fn validate_expected_reason(expected_reason: &str) -> Vec<String> {
    if expected_reason.trim().is_empty() {
        vec!["the expected reason code must not be empty".to_owned()]
    } else {
        Vec::new()
    }
}

fn validate_path_name(name: &str, names: &mut BTreeSet<String>, violations: &mut Vec<String>) {
    if name.trim().is_empty() {
        violations.push("a path name is empty".to_owned());
    } else if !names.insert(name.to_owned()) {
        violations.push(format!("path {name:?} is registered more than once"));
    }
}

fn finish(violations: Vec<String>) -> Result<(), LimitsConformanceError> {
    if violations.is_empty() {
        Ok(())
    } else {
        Err(LimitsConformanceError { violations })
    }
}

fn failure<Output>(message: impl Into<String>) -> Result<Output, LimitsConformanceError> {
    Err(LimitsConformanceError {
        violations: vec![message.into()],
    })
}

#[cfg(test)]
mod tests {
    use baukit_core::limits::{compact_json_utf8_bytes, trimmed_unicode_scalar_count};
    use serde_json::json;

    use super::*;

    #[derive(Clone, Debug)]
    struct FakeRow {
        deleted: bool,
        value: usize,
    }

    struct FakeRows {
        limit: usize,
        rows: Vec<FakeRow>,
        reject_updates: bool,
        retain_soft_deleted_rows: bool,
    }

    impl FakeRows {
        fn new(limit: usize) -> Self {
            Self {
                limit,
                rows: Vec::new(),
                reject_updates: false,
                retain_soft_deleted_rows: false,
            }
        }

        fn live_count(&self) -> usize {
            self.rows.iter().filter(|row| !row.deleted).count()
        }
    }

    impl LiveRowLimitAdapter for FakeRows {
        type Row = usize;
        type Error = &'static str;

        async fn create_row(&mut self, sequence: usize) -> Result<Self::Row, Self::Error> {
            if self.live_count() >= self.limit {
                return Err("row_cap_per_owner");
            }
            self.rows.push(FakeRow {
                deleted: false,
                value: sequence,
            });
            Ok(self.rows.len() - 1)
        }

        async fn update_row(&mut self, row: &Self::Row) -> Result<(), Self::Error> {
            if self.reject_updates {
                return Err("row_cap_per_owner");
            }
            self.rows[*row].value += 1;
            Ok(())
        }

        async fn soft_delete_row(&mut self, row: &Self::Row) -> Result<(), Self::Error> {
            if !self.retain_soft_deleted_rows {
                self.rows[*row].deleted = true;
            }
            Ok(())
        }
    }

    #[test]
    fn measures_trimmed_unicode_text_and_compact_document_bytes() {
        assert_eq!(trimmed_text_length("  é界  "), 2);
        assert_eq!(
            compact_document_bytes(&json!({"value": "é"})).expect("document should serialize"),
            r#"{"value":"é"}"#.len()
        );
    }

    #[test]
    fn compatibility_names_call_the_production_measurements() {
        let document = json!({"value": "é"});

        assert_eq!(
            trimmed_text_length("  e\u{301}  "),
            trimmed_unicode_scalar_count("  e\u{301}  ")
        );
        assert_eq!(
            compact_document_bytes(&document).expect("compatibility alias should encode"),
            compact_json_utf8_bytes(&document).expect("production helper should encode")
        );
    }

    #[tokio::test]
    async fn boundary_check_exercises_below_at_and_above() {
        let result = check_limit_boundaries(
            2,
            |length| "é".repeat(length),
            |text| async move {
                if trimmed_text_length(&text) <= 2 {
                    Ok(())
                } else {
                    Err("text_too_long")
                }
            },
        )
        .await;

        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn boundary_check_reports_a_broken_validator() {
        let error = check_limit_boundaries(2, |length| length, |_| async { Ok::<_, ()>(()) })
            .await
            .expect_err("a validator that accepts limit + 1 must fail conformance");

        assert_eq!(error.violations(), ["the limit + 1 payload was accepted"]);
    }

    #[tokio::test]
    async fn update_succeeds_at_live_row_capacity() {
        let mut rows = FakeRows::new(2);

        assert_eq!(check_update_at_capacity(&mut rows, 2).await, Ok(()));
        assert_eq!(rows.live_count(), 2);
        assert_eq!(rows.rows[0].value, 1);
    }

    #[tokio::test]
    async fn update_check_reports_a_broken_adapter() {
        let mut rows = FakeRows::new(2);
        rows.reject_updates = true;

        let error = check_update_at_capacity(&mut rows, 2)
            .await
            .expect_err("an update rejected at capacity must fail conformance");

        assert_eq!(
            error.violations(),
            ["updating a live row at capacity failed"]
        );
    }

    #[tokio::test]
    async fn soft_delete_releases_live_row_capacity() {
        let mut rows = FakeRows::new(2);

        assert_eq!(check_soft_delete_capacity_reuse(&mut rows, 2).await, Ok(()));
        assert_eq!(rows.live_count(), 2);
        assert!(rows.rows[0].deleted);
    }

    #[tokio::test]
    async fn soft_delete_check_reports_a_broken_adapter() {
        let mut rows = FakeRows::new(2);
        rows.retain_soft_deleted_rows = true;

        let error = check_soft_delete_capacity_reuse(&mut rows, 2)
            .await
            .expect_err("a retained soft-deleted row must fail conformance");

        assert_eq!(
            error.violations(),
            ["creating a row after soft deletion failed"]
        );
    }

    #[test]
    fn reason_code_check_reports_mismatched_error_shapes() {
        #[derive(Clone, Copy)]
        struct Output {
            reason: Option<&'static str>,
        }

        let error = check_reason_code_conformance(
            "too_many_elements",
            [
                NamedOutput::new(
                    "rest",
                    Output {
                        reason: Some("too_many_elements"),
                    },
                ),
                NamedOutput::new(
                    "local",
                    Output {
                        reason: Some("validation_failed"),
                    },
                ),
                NamedOutput::new("import", Output { reason: None }),
            ],
            |output| output.reason,
        )
        .expect_err("mismatched and missing reason codes must fail conformance");

        assert_eq!(
            error.violations(),
            [
                "path \"local\" returned reason code \"validation_failed\"; expected \"too_many_elements\"",
                "path \"import\" returned no reason code; expected \"too_many_elements\"",
            ]
        );
    }

    #[tokio::test]
    async fn ingress_check_invokes_every_path_and_reports_parity_failure() {
        #[derive(Clone, Copy)]
        struct Output(&'static str);

        let error = check_ingress_reason_code_parity(
            "jsonb_too_large",
            vec![
                NamedIngress::new("rest", || async { Output("jsonb_too_large") }),
                NamedIngress::new("sync", || async { Output("jsonb_too_large") }),
                NamedIngress::new("import", || async { Output("body_too_large") }),
                NamedIngress::new("local", || async { Output("jsonb_too_large") }),
            ],
            |output| Some(output.0),
        )
        .await
        .expect_err("one divergent ingress must fail conformance");

        assert_eq!(
            error.violations(),
            [
                "path \"import\" returned reason code \"body_too_large\"; expected \"jsonb_too_large\""
            ]
        );
    }

    #[tokio::test]
    async fn ingress_check_accepts_matching_reason_codes() {
        #[derive(Clone, Copy)]
        struct Output(&'static str);

        let result = check_ingress_reason_code_parity(
            "row_cap_per_owner",
            vec![
                NamedIngress::new("rest", || async { Output("row_cap_per_owner") }),
                NamedIngress::new("sync", || async { Output("row_cap_per_owner") }),
                NamedIngress::new("import", || async { Output("row_cap_per_owner") }),
                NamedIngress::new("local", || async { Output("row_cap_per_owner") }),
            ],
            |output| Some(output.0),
        )
        .await;

        assert_eq!(result, Ok(()));
    }
}
