use std::fmt;

use serde_json::Value;

use crate::security_probe::{
    Operation, OperationFixtures, RouteExclusion, SecurityTarget, operations, substitute,
    validate_exclusions,
};

const DEFAULT_DENIED_STATUSES: [u16; 2] = [403, 404];

/// Principal B's view of principal A's resources.
///
/// The [`OperationFixtures`] path values are A's ids. `token` is B's bearer
/// token. Every operation whose path has a parameter is called as B with A's
/// ids and must answer with one of the denied statuses (403 or 404 by
/// default) and without any leak marker in the body.
#[derive(Clone)]
pub struct CrossTenantCase {
    token: String,
    denied_statuses: Vec<u16>,
    exclusions: Vec<RouteExclusion>,
    leak_markers: Vec<String>,
}

impl fmt::Debug for CrossTenantCase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CrossTenantCase")
            .field("token", &"<redacted>")
            .field("denied_statuses", &self.denied_statuses)
            .field("exclusions", &self.exclusions)
            .field("leak_markers", &self.leak_markers)
            .finish()
    }
}

impl CrossTenantCase {
    /// Probes as the principal that owns `token`.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            denied_statuses: DEFAULT_DENIED_STATUSES.to_vec(),
            exclusions: Vec::new(),
            leak_markers: Vec::new(),
        }
    }

    /// Replaces the statuses that count as a denial.
    #[must_use]
    pub fn denied_statuses(mut self, statuses: impl IntoIterator<Item = u16>) -> Self {
        self.denied_statuses = statuses.into_iter().collect();
        self
    }

    /// Leaves `method path` out of the probe. The reason is printed on every run.
    #[must_use]
    pub fn exclude(
        mut self,
        method: &str,
        path: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        self.exclusions
            .push(RouteExclusion::new(method, path, reason));
        self
    }

    /// Fails any response whose body contains `marker`, such as A's item title.
    #[must_use]
    pub fn leak_marker(mut self, marker: impl Into<String>) -> Self {
        self.leak_markers.push(marker.into());
        self
    }
}

/// What a passing cross-tenant probe covered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossTenantReport {
    /// Operations called as principal B, as `METHOD /path`.
    pub probed: Vec<String>,
    /// Operations left out, with their reasons.
    pub excluded: Vec<RouteExclusion>,
}

impl fmt::Display for CrossTenantReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "cross-tenant probe denied {} operation(s)",
            self.probed.len()
        )?;
        for exclusion in &self.excluded {
            writeln!(
                formatter,
                "excluded {}: {}",
                exclusion.label(),
                exclusion.reason
            )?;
        }
        Ok(())
    }
}

/// Routes that let principal B reach principal A's resources.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossTenantError {
    violations: Vec<String>,
}

impl CrossTenantError {
    /// Returns one entry per failing operation, in document order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for CrossTenantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "cross-tenant access was not denied:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CrossTenantError {}

/// Calls every parameterized operation in `document` as principal B with
/// principal A's ids and collects each one that was not denied.
///
/// `document` is the product's OpenAPI JSON, for example
/// `serde_json::to_value(ApiDoc::openapi())`. An operation with a path
/// parameter that has no fixture value is a violation, so a new route fails
/// the test until it gets a fixture or an exclusion.
///
/// A product test that seeds one item for Alice and probes it as Bob, then
/// runs the auth negative suite over the same document:
///
/// ```no_run
/// # async fn product_test(
/// #     router: axum::Router,
/// #     document: serde_json::Value,
/// #     issuer: &baukit_test::MockOidcServer,
/// #     alice_item_id: &str,
/// #     bob_token: String,
/// # ) {
/// use baukit_test::{
///     AuthNegativeCase, CrossTenantCase, OperationFixtures, SecurityTarget,
///     assert_auth_negative_suite, assert_cross_tenant_denied,
/// };
///
/// let target = SecurityTarget::router(router);
/// let fixtures = OperationFixtures::new()
///     .path_value("itemId", alice_item_id)
///     .body("put", "/v1/items/{itemId}", serde_json::json!({"title": "bob"}))
///     .header("idempotency-key", "cross-tenant-probe");
/// let case = CrossTenantCase::new(bob_token)
///     .leak_marker("alice's private title")
///     .exclude("get", "/v1/shares/{shareId}", "share links are readable by any signed-in user");
/// assert_cross_tenant_denied(&document, &target, &fixtures, &case).await;
/// assert_auth_negative_suite(&document, &target, &fixtures, &AuthNegativeCase::new(issuer, "api"))
///     .await;
/// # }
/// ```
///
/// # Errors
///
/// Returns [`CrossTenantError`] with one violation per failing operation: an
/// unreadable document, an exclusion naming no operation, a missing fixture, or a
/// route that lets the second principal reach the first one's resource.
pub async fn check_cross_tenant_denied(
    document: &Value,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    case: &CrossTenantCase,
) -> Result<CrossTenantReport, CrossTenantError> {
    let all = operations(document).map_err(|message| CrossTenantError {
        violations: vec![message],
    })?;
    let mut violations = Vec::new();
    validate_exclusions(&case.exclusions, &all, &mut violations);
    let mut probed = Vec::new();
    for operation in all
        .iter()
        .filter(|operation| !operation.path_parameters().is_empty())
    {
        if case
            .exclusions
            .iter()
            .any(|exclusion| exclusion.matches(operation))
        {
            continue;
        }
        probed.push(operation.label());
        if let Some(violation) = probe(operation, target, fixtures, case).await {
            violations.push(violation);
        }
    }
    if !violations.is_empty() {
        return Err(CrossTenantError { violations });
    }
    Ok(CrossTenantReport {
        probed,
        excluded: case.exclusions.clone(),
    })
}

/// Panics listing every operation that let principal B reach A's resources.
///
/// Prints the probed count and every exclusion with its reason.
///
/// # Panics
///
/// Panics with all violations found by [`check_cross_tenant_denied`].
pub async fn assert_cross_tenant_denied(
    document: &Value,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    case: &CrossTenantCase,
) -> CrossTenantReport {
    match check_cross_tenant_denied(document, target, fixtures, case).await {
        Ok(report) => {
            println!("{report}");
            report
        }
        Err(error) => panic!("{error}"),
    }
}

async fn probe(
    operation: &Operation,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    case: &CrossTenantCase,
) -> Option<String> {
    let label = operation.label();
    let uri = match substitute(&operation.path, fixtures.path_values()) {
        Ok(uri) => uri,
        Err(message) => return Some(format!("{label}: {message}")),
    };
    let request = fixtures.request(operation, &uri, Some(&case.token));
    let response = match target.send(request).await {
        Ok(response) => response,
        Err(message) => return Some(format!("{label}: request failed: {message}")),
    };
    if let Some(marker) = case
        .leak_markers
        .iter()
        .find(|marker| response.body.contains(marker.as_str()))
    {
        return Some(format!(
            "{label}: status {} leaked `{marker}` in the body",
            response.status
        ));
    }
    if case.denied_statuses.contains(&response.status) {
        return None;
    }
    Some(format!(
        "{label}: status {}, expected one of {:?}",
        response.status, case.denied_statuses
    ))
}
