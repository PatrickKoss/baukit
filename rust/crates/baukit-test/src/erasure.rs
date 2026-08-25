use std::{collections::HashSet, fmt};

/// How a product removes one class of subject-owned resources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupKind {
    /// A foreign key deletes the resource with its owning row.
    Cascade,
    /// Product erasure code deletes the resource explicitly.
    Explicit,
    /// A registered background processor completes deletion.
    AsyncProcessor,
}

/// One product-maintained check for a class of subject-owned resources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedResourceCheck {
    /// Stable resource name, normally its database table name.
    pub name: &'static str,
    /// Product SQL that counts rows owned by the bound subject.
    pub count_sql: &'static str,
    /// Declared deletion mechanism.
    pub cleanup: CleanupKind,
}

/// Product adapter exercised by the erasure conformance harness.
#[allow(async_fn_in_trait)]
pub trait ProductProfileErasureAdapter {
    /// Adapter-specific failure type. Its message is not included in harness output.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Creates a representative owned-resource graph for `subject`.
    async fn seed_user_owned_resource_graph(&mut self, subject: &str) -> Result<(), Self::Error>;

    /// Explains why the seed cannot create a row for one cascade or explicit check.
    fn unseeded_resource_reason(&self, _resource: &OwnedResourceCheck) -> Option<&'static str> {
        None
    }

    /// Counts resources selected by one registry entry for `subject`.
    async fn owned_resource_count(
        &self,
        subject: &str,
        resource: &OwnedResourceCheck,
    ) -> Result<u64, Self::Error>;

    /// Invokes the product's authoritative profile-erasure operation.
    async fn erase_product_profile(&mut self, subject: &str) -> Result<(), Self::Error>;

    /// Counts registered background jobs that still contain work for `subject`.
    async fn registered_background_job_count(&self, subject: &str) -> Result<u64, Self::Error>;
}

/// Failure reported by the product-profile erasure conformance harness.
#[derive(Debug, Eq, PartialEq)]
pub struct ErasureConformanceError {
    violations: Vec<String>,
}

impl ErasureConformanceError {
    /// Returns every observed contract violation.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for ErasureConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "product-profile erasure conformance failed: {}",
            self.violations.join("; ")
        )
    }
}

impl std::error::Error for ErasureConformanceError {}

/// Seeds, erases, and verifies a product adapter, then repeats the erasure.
pub async fn check_product_profile_erasure_conformance<TAdapter>(
    adapter: &mut TAdapter,
    subject: &str,
    resources: &[OwnedResourceCheck],
) -> Result<(), ErasureConformanceError>
where
    TAdapter: ProductProfileErasureAdapter,
{
    let mut violations = validate_registry(subject, resources);
    if !violations.is_empty() {
        return Err(ErasureConformanceError { violations });
    }
    if adapter
        .seed_user_owned_resource_graph(subject)
        .await
        .is_err()
    {
        return Err(ErasureConformanceError {
            violations: vec!["seeding the subject-owned resource graph failed".to_owned()],
        });
    }
    collect_seeded_resources(adapter, subject, resources, &mut violations).await;
    if !violations.is_empty() {
        return Err(ErasureConformanceError { violations });
    }
    if adapter.erase_product_profile(subject).await.is_err() {
        return Err(ErasureConformanceError {
            violations: vec!["the first product-profile erasure invocation failed".to_owned()],
        });
    }
    collect_remaining_resources(
        adapter,
        subject,
        resources,
        "after the first erasure",
        &mut violations,
    )
    .await;

    if adapter.erase_product_profile(subject).await.is_err() {
        violations.push("the repeated product-profile erasure invocation failed".to_owned());
    } else {
        collect_remaining_resources(
            adapter,
            subject,
            resources,
            "after the repeated erasure",
            &mut violations,
        )
        .await;
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(ErasureConformanceError { violations })
    }
}

fn validate_registry(subject: &str, resources: &[OwnedResourceCheck]) -> Vec<String> {
    let mut violations = Vec::new();
    if subject.trim().is_empty() {
        violations.push("the conformance subject must not be empty".to_owned());
    }
    if resources.is_empty() {
        violations.push("the owned-resource registry must not be empty".to_owned());
    }
    let mut names = HashSet::new();
    for resource in resources {
        if resource.name.trim().is_empty() {
            violations.push("an owned-resource name is empty".to_owned());
        } else if !names.insert(resource.name) {
            violations.push(format!(
                "owned resource {:?} is registered more than once",
                resource.name
            ));
        }
        if resource.count_sql.trim().is_empty() {
            violations.push(format!(
                "owned resource {:?} has no count SQL",
                resource.name
            ));
        } else if !has_subject_binding(resource.count_sql) {
            violations.push(format!(
                "owned resource {:?} count SQL has no $1 subject binding",
                resource.name
            ));
        }
    }
    violations
}

fn has_subject_binding(sql: &str) -> bool {
    sql.match_indices("$1").any(|(index, _)| {
        sql[index + 2..]
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_digit())
    })
}

async fn collect_seeded_resources<TAdapter>(
    adapter: &TAdapter,
    subject: &str,
    resources: &[OwnedResourceCheck],
    violations: &mut Vec<String>,
) where
    TAdapter: ProductProfileErasureAdapter,
{
    let mut all_counts_known = true;
    let mut any_resource_seeded = false;
    for resource in resources {
        match adapter.owned_resource_count(subject, resource).await {
            Ok(count) => {
                any_resource_seeded |= count > 0;
                let requires_row = matches!(
                    resource.cleanup,
                    CleanupKind::Cascade | CleanupKind::Explicit
                );
                let has_reason = adapter
                    .unseeded_resource_reason(resource)
                    .is_some_and(|reason| !reason.trim().is_empty());
                if count == 0 && requires_row && !has_reason {
                    violations.push(format!(
                        "owned resource {:?} was not seeded before erasure",
                        resource.name
                    ));
                }
            }
            Err(_) => {
                all_counts_known = false;
                violations.push(format!(
                    "counting owned resource {:?} failed before erasure",
                    resource.name
                ));
            }
        }
    }
    if all_counts_known && !any_resource_seeded {
        violations
            .push("every registered owned resource was already zero after seeding".to_owned());
    }
    if adapter
        .registered_background_job_count(subject)
        .await
        .is_err()
    {
        violations.push("counting registered background jobs failed before erasure".to_owned());
    }
}

async fn collect_remaining_resources<TAdapter>(
    adapter: &TAdapter,
    subject: &str,
    resources: &[OwnedResourceCheck],
    phase: &str,
    violations: &mut Vec<String>,
) where
    TAdapter: ProductProfileErasureAdapter,
{
    for resource in resources {
        match adapter.owned_resource_count(subject, resource).await {
            Ok(0) => {}
            Ok(count) => violations.push(format!(
                "owned resource {:?} retained {count} row(s) {phase}",
                resource.name
            )),
            Err(_) => violations.push(format!(
                "counting owned resource {:?} failed {phase}",
                resource.name
            )),
        }
    }
    match adapter.registered_background_job_count(subject).await {
        Ok(0) => {}
        Ok(count) => violations.push(format!(
            "registered background jobs retained {count} item(s) {phase}"
        )),
        Err(_) => violations.push(format!(
            "counting registered background jobs failed {phase}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::HashMap, convert::Infallible};

    use super::*;

    const RESOURCES: [OwnedResourceCheck; 2] = [
        OwnedResourceCheck {
            name: "profiles",
            count_sql: "SELECT count(*) FROM profiles WHERE subject = $1",
            cleanup: CleanupKind::Cascade,
        },
        OwnedResourceCheck {
            name: "exports",
            count_sql: "SELECT count(*) FROM exports WHERE subject = $1",
            cleanup: CleanupKind::Explicit,
        },
    ];

    struct MemoryAdapter {
        counts: HashMap<&'static str, u64>,
        jobs: u64,
        leak: Option<&'static str>,
        seed_resources: bool,
        omitted_resource: Option<&'static str>,
        omission_reason: Option<&'static str>,
        erasures: usize,
        resource_count_calls: Cell<usize>,
        job_count_calls: Cell<usize>,
    }

    impl MemoryAdapter {
        fn new(leak: Option<&'static str>) -> Self {
            Self {
                counts: HashMap::new(),
                jobs: 0,
                leak,
                seed_resources: true,
                omitted_resource: None,
                omission_reason: None,
                erasures: 0,
                resource_count_calls: Cell::new(0),
                job_count_calls: Cell::new(0),
            }
        }

        fn without_seed() -> Self {
            Self {
                seed_resources: false,
                ..Self::new(None)
            }
        }

        fn omitting(resource: &'static str, reason: Option<&'static str>) -> Self {
            Self {
                omitted_resource: Some(resource),
                omission_reason: reason,
                ..Self::new(None)
            }
        }
    }

    impl ProductProfileErasureAdapter for MemoryAdapter {
        type Error = Infallible;

        async fn seed_user_owned_resource_graph(
            &mut self,
            _subject: &str,
        ) -> Result<(), Self::Error> {
            if self.seed_resources {
                self.counts = RESOURCES
                    .iter()
                    .filter(|resource| self.omitted_resource != Some(resource.name))
                    .map(|resource| (resource.name, 1))
                    .collect();
                self.jobs = 1;
            }
            Ok(())
        }

        fn unseeded_resource_reason(&self, resource: &OwnedResourceCheck) -> Option<&'static str> {
            (self.omitted_resource == Some(resource.name))
                .then_some(self.omission_reason)
                .flatten()
        }

        async fn owned_resource_count(
            &self,
            _subject: &str,
            resource: &OwnedResourceCheck,
        ) -> Result<u64, Self::Error> {
            self.resource_count_calls
                .set(self.resource_count_calls.get() + 1);
            Ok(self.counts.get(resource.name).copied().unwrap_or_default())
        }

        async fn erase_product_profile(&mut self, _subject: &str) -> Result<(), Self::Error> {
            self.erasures += 1;
            for resource in &RESOURCES {
                if self.leak != Some(resource.name) {
                    self.counts.insert(resource.name, 0);
                }
            }
            self.jobs = 0;
            Ok(())
        }

        async fn registered_background_job_count(
            &self,
            _subject: &str,
        ) -> Result<u64, Self::Error> {
            self.job_count_calls.set(self.job_count_calls.get() + 1);
            Ok(self.jobs)
        }
    }

    #[tokio::test]
    async fn accepts_complete_and_repeatable_erasure() {
        let mut adapter = MemoryAdapter::new(None);

        check_product_profile_erasure_conformance(&mut adapter, "subject-a", &RESOURCES)
            .await
            .expect("complete erasure should conform");

        assert_eq!(adapter.erasures, 2);
        assert!(adapter.counts.values().all(|count| *count == 0));
        assert_eq!(adapter.jobs, 0);
        assert_eq!(adapter.resource_count_calls.get(), RESOURCES.len() * 3);
        assert_eq!(adapter.job_count_calls.get(), 3);
    }

    #[tokio::test]
    async fn rejects_a_leaky_owned_resource() {
        let mut adapter = MemoryAdapter::new(Some("exports"));

        let error =
            check_product_profile_erasure_conformance(&mut adapter, "subject-a", &RESOURCES)
                .await
                .expect_err("a retained resource must fail conformance");

        assert!(
            error
                .violations()
                .iter()
                .any(|violation| violation.contains("exports") && violation.contains("retained"))
        );
    }

    #[tokio::test]
    async fn rejects_an_adapter_that_seeds_no_owned_resources() {
        let mut adapter = MemoryAdapter::without_seed();

        let error =
            check_product_profile_erasure_conformance(&mut adapter, "subject-a", &RESOURCES)
                .await
                .expect_err("an empty seed must fail conformance");

        assert!(error.violations().iter().any(|violation| violation
            == "every registered owned resource was already zero after seeding"));
        assert_eq!(adapter.erasures, 0);
    }

    #[tokio::test]
    async fn requires_each_cascade_and_explicit_resource_or_a_reason() {
        for resource in &RESOURCES {
            let mut adapter = MemoryAdapter::omitting(resource.name, None);
            let error =
                check_product_profile_erasure_conformance(&mut adapter, "subject-a", &RESOURCES)
                    .await
                    .expect_err("an unexplained seed omission must fail conformance");
            assert!(error.violations().iter().any(|violation| {
                violation.contains(resource.name) && violation.contains("was not seeded")
            }));
        }

        let mut adapter = MemoryAdapter::omitting("profiles", Some("external fixture only"));
        check_product_profile_erasure_conformance(&mut adapter, "subject-a", &RESOURCES)
            .await
            .expect("a declared seed omission should conform when another resource is seeded");
    }

    #[tokio::test]
    async fn rejects_count_sql_without_the_subject_binding() {
        for count_sql in [
            "SELECT count(*) FROM profiles",
            "SELECT count(*) FROM profiles WHERE subject = $10",
        ] {
            let resources = [OwnedResourceCheck {
                count_sql,
                ..RESOURCES[0]
            }];
            let mut adapter = MemoryAdapter::new(None);

            let error =
                check_product_profile_erasure_conformance(&mut adapter, "subject-a", &resources)
                    .await
                    .expect_err("count SQL without $1 must fail conformance");

            assert!(
                error
                    .violations()
                    .iter()
                    .any(|violation| violation.contains("no $1 subject binding"))
            );
            assert_eq!(adapter.erasures, 0);
        }
    }
}
