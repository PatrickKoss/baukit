#![cfg(test)]
#![cfg(feature = "platform")]
use async_trait::async_trait;
use baukit_suite::platform::*;
use jiff::Timestamp;
use proptest::prelude::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};
use uuid::Uuid;

const PRODUCT: &str = "bv";
const ALICE: &str = "suite-test|alice";
const STUDIO: Uuid = Uuid::from_u128(0x0a);
const START: i64 = 1_800_000_000;

struct TestClock(AtomicI64);
impl PlatformClock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0.load(Ordering::SeqCst)).expect("in range")
    }
}

/// The hub's answer for Alice in the studio; `None` means not a member.
struct Directory(Mutex<Option<OrganizationRole>>);
#[async_trait]
impl OrganizationDirectory for Directory {
    async fn summary(&self, _: Uuid) -> Result<OrganizationSummary, OrganizationError> {
        Err(OrganizationError::NotFound)
    }
}
#[async_trait]
impl OrganizationGrants for Directory {
    async fn subject_organizations(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<SubjectOrganizationPage, OrganizationError> {
        let role = *self.0.lock().expect("role");
        Ok(SubjectOrganizationPage {
            items: role
                .map(|account_role| SubjectOrganization {
                    organization_id: STUDIO,
                    account_role,
                })
                .into_iter()
                .collect(),
            next_cursor: None,
        })
    }
}

struct NoTickets;
#[async_trait]
impl TicketSource for NoTickets {
    async fn ticket(&self, _: &str) -> Result<String, EntitlementSourceError> {
        Err(EntitlementSourceError::Unavailable)
    }
}
struct NoKeys;
#[async_trait]
impl TicketJwksSource for NoKeys {
    async fn fetch(&self) -> Result<TicketJwks, TicketError> {
        Err(TicketError::KeysUnavailable)
    }
}
struct NoCredits;
#[async_trait]
impl Credits for NoCredits {
    async fn reserve(&self, _: &str, _: u64, _: &str) -> Result<CreditHold, CreditsError> {
        Err(CreditsError::Unavailable)
    }
    async fn settle(&self, _: Uuid, _: u64) -> Result<CreditReservation, CreditsError> {
        Err(CreditsError::Unavailable)
    }
    async fn release(&self, _: Uuid) -> Result<CreditReservation, CreditsError> {
        Err(CreditsError::Unavailable)
    }
    async fn balance(&self, _: &str) -> Result<CreditBalance, CreditsError> {
        Err(CreditsError::Unavailable)
    }
}

/// A hub whose membership read answers Admin only after the test opens the gate,
/// so the answer lands after a removal event evicted the member.
struct GatedHub {
    started: tokio::sync::Notify,
    gate: tokio::sync::Notify,
}
#[async_trait]
impl OrganizationDirectory for GatedHub {
    async fn summary(&self, _: Uuid) -> Result<OrganizationSummary, OrganizationError> {
        Err(OrganizationError::NotFound)
    }
}
#[async_trait]
impl OrganizationGrants for GatedHub {
    async fn subject_organizations(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<SubjectOrganizationPage, OrganizationError> {
        self.started.notify_one();
        self.gate.notified().await;
        Ok(SubjectOrganizationPage {
            items: vec![SubjectOrganization {
                organization_id: STUDIO,
                account_role: OrganizationRole::Admin,
            }],
            next_cursor: None,
        })
    }
}

fn plan(directory: Arc<Directory>, clock: Arc<TestClock>) -> HubAccountPlan {
    plan_with(directory, clock)
}

fn plan_with(directory: Arc<dyn OrganizationGrants>, clock: Arc<TestClock>) -> HubAccountPlan {
    let verifier = TicketVerifier::with_source(
        "https://accounts.example.test".into(),
        PRODUCT.into(),
        Arc::new(NoKeys),
    )
    .expect("verifier");
    HubAccountPlan::new(
        PRODUCT.into(),
        VerifiedTicketSource::new(Arc::new(NoTickets), verifier),
        Arc::new(InMemoryEntitlementCache::default()),
        Arc::new(NoCredits),
        "https://accounts.example.test/plans",
    )
    .expect("plan")
    .with_organizations(directory)
    .with_clock(clock)
}

fn role() -> impl Strategy<Value = OrganizationRole> {
    prop_oneof![
        Just(OrganizationRole::Owner),
        Just(OrganizationRole::Admin),
        Just(OrganizationRole::Billing),
        Just(OrganizationRole::Member),
    ]
}

#[derive(Clone, Debug)]
enum Step {
    Advance(i64),
    /// The hub removes Alice; `notify` says whether the event reaches this replica.
    Remove {
        notify: bool,
    },
    Add(OrganizationRole),
    Check,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0..=90_i64).prop_map(Step::Advance),
        any::<bool>().prop_map(|notify| Step::Remove { notify }),
        role().prop_map(Step::Add),
        Just(Step::Check),
    ]
}

/// What the replica may answer: the hub's truth, or a stale member answer that
/// it cached less than a TTL ago and no event evicted.
#[derive(Default)]
struct Model {
    hub: Option<OrganizationRole>,
    stale_until: Option<(OrganizationRole, i64)>,
}

async fn run(steps: Vec<Step>) -> Result<(), TestCaseError> {
    let directory = Arc::new(Directory(Mutex::new(None)));
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let plan = plan(directory.clone(), clock.clone());
    let mut model = Model::default();
    for step in steps {
        let now = clock.0.load(Ordering::SeqCst);
        match step {
            Step::Advance(seconds) => {
                clock.0.fetch_add(seconds, Ordering::SeqCst);
            }
            Step::Remove { notify } => {
                *directory.0.lock().expect("role") = None;
                model.hub = None;
                if notify {
                    plan.evict_member(STUDIO, ALICE).await;
                    model.stale_until = None;
                }
            }
            Step::Add(role) => {
                *directory.0.lock().expect("role") = Some(role);
                model.hub = Some(role);
                plan.evict_member(STUDIO, ALICE).await;
                model.stale_until = None;
            }
            Step::Check => {
                let answer = plan.membership(STUDIO, ALICE).await.expect("membership");
                let stale = model
                    .stale_until
                    .filter(|(_, until)| now < *until)
                    .map(|(role, _)| role);
                prop_assert!(answer == model.hub || answer == stale);
                if model.hub.is_none() && stale.is_none() {
                    prop_assert!(!plan.is_member(STUDIO, ALICE).await.expect("membership"));
                }
                if let Some(role) = answer
                    && stale.is_none()
                {
                    model.stale_until = Some((role, now + ORGANIZATION_CACHE_TTL_SECONDS));
                }
                if answer == Some(OrganizationRole::Billing) {
                    prop_assert_eq!(
                        require_role(&plan, STUDIO, Some(ALICE.into()), OrganizationRole::Member)
                            .await,
                        Err(PlanError::OrganizationNotAccessible)
                    );
                }
            }
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn membership_follows_events_and_expires(steps in prop::collection::vec(step(), 1..40)) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        runtime.block_on(run(steps))?;
    }

    #[test]
    fn billing_never_satisfies_member(required in role()) {
        prop_assert_eq!(
            OrganizationRole::Billing.satisfies(required),
            required == OrganizationRole::Billing
        );
    }
}

#[tokio::test]
async fn a_removal_event_ends_access_on_this_replica() {
    let directory = Arc::new(Directory(Mutex::new(Some(OrganizationRole::Admin))));
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let plan: Arc<dyn AccountPlan> = Arc::new(plan(directory.clone(), clock));
    assert_eq!(
        require_role(
            plan.as_ref(),
            STUDIO,
            Some(ALICE.into()),
            OrganizationRole::Member
        )
        .await,
        Ok(OrganizationRole::Admin)
    );
    *directory.0.lock().expect("role") = None;
    assert!(plan.is_member(STUDIO, ALICE).await.expect("cached"));

    let handler = MembershipEvictionHandler::new(plan.clone(), Arc::new(Ignore));
    let event = membership_removed();
    handler
        .handle(&event, &mut EventContext::detached())
        .await
        .expect("handled");
    assert!(!plan.is_member(STUDIO, ALICE).await.expect("membership"));
}

#[tokio::test]
async fn a_read_in_flight_during_removal_does_not_restore_the_member() {
    let hub = Arc::new(GatedHub {
        started: tokio::sync::Notify::new(),
        gate: tokio::sync::Notify::new(),
    });
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let plan: Arc<dyn AccountPlan> = Arc::new(plan_with(hub.clone(), clock));
    let reader = tokio::spawn({
        let plan = plan.clone();
        async move { plan.is_member(STUDIO, ALICE).await }
    });
    hub.started.notified().await;
    MembershipEvictionHandler::new(plan.clone(), Arc::new(Ignore))
        .handle(&membership_removed(), &mut EventContext::detached())
        .await
        .expect("handled");
    hub.gate.notify_one();
    assert!(!reader.await.expect("reader").expect("membership"));

    let again = tokio::spawn({
        let plan = plan.clone();
        async move { plan.is_member(STUDIO, ALICE).await }
    });
    hub.started.notified().await;
    hub.gate.notify_one();
    assert!(again.await.expect("reader").expect("membership"));
}

#[tokio::test]
async fn membership_expires_without_an_event() {
    let directory = Arc::new(Directory(Mutex::new(Some(OrganizationRole::Member))));
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let plan = plan(directory.clone(), clock.clone());
    assert!(plan.is_member(STUDIO, ALICE).await.expect("membership"));
    *directory.0.lock().expect("role") = None;
    clock
        .0
        .fetch_add(ORGANIZATION_CACHE_TTL_SECONDS - 1, Ordering::SeqCst);
    assert!(plan.is_member(STUDIO, ALICE).await.expect("cached"));
    clock.0.fetch_add(1, Ordering::SeqCst);
    assert!(!plan.is_member(STUDIO, ALICE).await.expect("expired"));
}

#[tokio::test]
async fn billing_cannot_act_as_a_member() {
    let directory = Arc::new(Directory(Mutex::new(Some(OrganizationRole::Billing))));
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let plan = plan(directory, clock);
    assert_eq!(
        require_role(&plan, STUDIO, Some(ALICE.into()), OrganizationRole::Member).await,
        Err(PlanError::OrganizationNotAccessible)
    );
    assert_eq!(
        require_role(&plan, STUDIO, Some(ALICE.into()), OrganizationRole::Billing).await,
        Ok(OrganizationRole::Billing)
    );
    assert_eq!(
        require_role(&plan, STUDIO, None, OrganizationRole::Billing).await,
        Err(PlanError::SuiteIdentityMissing)
    );
}

struct Ignore;
#[async_trait]
impl PlatformEventHandler for Ignore {
    async fn handle(
        &self,
        _: &PlatformEvent,
        _: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
}

fn membership_removed() -> PlatformEvent {
    let body = serde_json::json!({
        "eventId": "membership-removed-1",
        "type": "schluesselbund.organization.membership_changed",
        "userId": ALICE,
        "occurredAt": "2027-01-15T08:00:00Z",
        "sourceApp": "schluesselbund",
        "schemaVersion": 1,
        "payload": {
            "organizationId": STUDIO,
            "suiteSubject": ALICE,
            "status": "removed",
            "accountRole": "member"
        }
    });
    PlatformEventDecoder::new("schluesselbund".into(), None)
        .expect("decoder")
        .decode(
            &serde_json::to_vec(&body).expect("json"),
            "2027-01-15T08:00:00Z".parse().expect("time"),
        )
        .expect("event")
}
