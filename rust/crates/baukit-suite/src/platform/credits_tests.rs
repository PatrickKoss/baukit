//! P-16: the response checks against a scripted hub ledger.

use super::*;
use proptest::prelude::*;
use std::collections::BTreeMap;

const WALLET: &str = "hebkit";

#[derive(Clone, Debug)]
enum Op {
    Reserve { key: u8, amount: u64 },
    Settle { key: u8, actual: u64 },
    Release { key: u8 },
    Expire { key: u8 },
}

#[derive(Clone, Copy, Debug)]
enum Tamper {
    None,
    Status(ReservationStatus),
    Charge(u64),
    Amount(u64),
}

/// An honest hub ledger keyed by idempotency key.
#[derive(Default)]
struct Ledger {
    reservations: BTreeMap<u8, CreditReservation>,
}
impl Ledger {
    fn reserve(&mut self, key: u8, amount: u64) -> CreditReservation {
        self.reservations
            .entry(key)
            .or_insert_with(|| CreditReservation {
                id: Uuid::from_u128(u128::from(key)),
                wallet: WALLET.into(),
                amount,
                charged: None,
                status: ReservationStatus::Pending,
                expires_at: 0,
            })
            .clone()
    }
    fn finish(&mut self, key: u8, status: ReservationStatus, charged: Option<u64>) {
        if let Some(r) = self.reservations.get_mut(&key)
            && r.status == ReservationStatus::Pending
        {
            r.status = status;
            r.charged = charged;
        }
    }
}

fn apply(tamper: Tamper, mut r: CreditReservation) -> CreditReservation {
    match tamper {
        Tamper::None => {}
        Tamper::Status(status) => r.status = status,
        Tamper::Charge(charged) => r.charged = Some(charged),
        Tamper::Amount(amount) => r.amount = amount,
    }
    r
}

fn op() -> impl Strategy<Value = Op> {
    let key = 0..4_u8;
    prop_oneof![
        (key.clone(), 1..20_u64).prop_map(|(key, amount)| Op::Reserve { key, amount }),
        (key.clone(), 0..30_u64).prop_map(|(key, actual)| Op::Settle { key, actual }),
        key.clone().prop_map(|key| Op::Release { key }),
        key.prop_map(|key| Op::Expire { key }),
    ]
}

fn status() -> impl Strategy<Value = ReservationStatus> {
    prop_oneof![
        Just(ReservationStatus::Pending),
        Just(ReservationStatus::Settled),
        Just(ReservationStatus::Released),
        Just(ReservationStatus::Expired),
    ]
}

fn tamper() -> impl Strategy<Value = Tamper> {
    prop_oneof![
        4 => Just(Tamper::None),
        1 => status().prop_map(Tamper::Status),
        1 => (0..40_u64).prop_map(Tamper::Charge),
        1 => (1..40_u64).prop_map(Tamper::Amount),
    ]
}

proptest! {
    /// Each accepted reservation is finished at most once, accepted charges
    /// never exceed accepted reservations, and a replay after settle is refused.
    #[test]
    fn accepted_answers_follow_the_reservation_state_machine(
        steps in prop::collection::vec((op(), tamper()), 1..60)
    ) {
        let mut ledger = Ledger::default();
        let mut reserved: BTreeMap<u8, u64> = BTreeMap::new();
        let mut finished: BTreeMap<u8, ReservationStatus> = BTreeMap::new();
        let mut charged_total = 0_u64;
        for (op, tamper) in steps {
            match op {
                Op::Reserve { key, amount } => {
                    let first = ledger.reserve(key, amount);
                    let answer = checked_reservation(apply(tamper, first.clone()), amount);
                    if let Ok(r) = &answer {
                        prop_assert_eq!(r.status, ReservationStatus::Pending);
                        prop_assert_eq!(r.amount, amount);
                        // A stateless client cannot catch a hub that relabels a finished
                        // reservation, so only honest answers must respect `finished`.
                        if matches!(tamper, Tamper::None) {
                            prop_assert!(!finished.contains_key(&key));
                        }
                        reserved.entry(key).or_insert(amount);
                    }
                    if matches!(tamper, Tamper::None) {
                        let expected = match first.status {
                            _ if first.amount != amount => Err(invalid_response()),
                            ReservationStatus::Pending => Ok(first.clone()),
                            ReservationStatus::Expired => Err(CreditsError::ReservationExpired),
                            _ => Err(CreditsError::AlreadySettled),
                        };
                        prop_assert_eq!(answer, expected);
                    }
                }
                Op::Settle { key, actual } => {
                    let Some(r) = ledger.reservations.get(&key).cloned() else { continue };
                    if r.status != ReservationStatus::Pending {
                        continue;
                    }
                    let charge = actual.min(r.amount);
                    ledger.finish(key, ReservationStatus::Settled, Some(charge));
                    let honest = ledger.reservations[&key].clone();
                    if let Ok(settled) = checked_settlement(apply(tamper, honest), actual) {
                        let charged = settled.charged.unwrap_or_default();
                        prop_assert!(charged <= settled.amount && charged <= actual);
                        if reserved.contains_key(&key) && !finished.contains_key(&key) {
                            prop_assert!(charged <= reserved[&key] || settled.amount != reserved[&key]);
                            charged_total += charged.min(reserved[&key]);
                            finished.insert(key, ReservationStatus::Settled);
                        }
                    }
                }
                Op::Release { key } => {
                    if ledger.reservations.get(&key).is_none_or(|r| r.status != ReservationStatus::Pending) {
                        continue;
                    }
                    ledger.finish(key, ReservationStatus::Released, None);
                    let honest = ledger.reservations[&key].clone();
                    if let Ok(released) = checked_release(apply(tamper, honest)) {
                        prop_assert_eq!(released.status, ReservationStatus::Released);
                        prop_assert!(released.charged.unwrap_or_default() == 0);
                        finished.entry(key).or_insert(ReservationStatus::Released);
                    }
                }
                Op::Expire { key } => ledger.finish(key, ReservationStatus::Expired, None),
            }
        }
        prop_assert!(charged_total <= reserved.values().sum::<u64>());
    }
}

#[test]
fn a_replayed_key_after_settle_is_refused() {
    let mut ledger = Ledger::default();
    let first = ledger.reserve(1, 4);
    assert_eq!(
        checked_reservation(first, 4).map(|r| r.status),
        Ok(ReservationStatus::Pending)
    );
    ledger.finish(1, ReservationStatus::Settled, Some(4));
    assert_eq!(
        checked_reservation(ledger.reserve(1, 4), 4),
        Err(CreditsError::AlreadySettled)
    );
    assert_eq!(
        checked_reservation(ledger.reserve(1, 5), 5),
        Err(invalid_response()),
        "another amount is not this request's reservation"
    );
}

#[test]
fn settle_and_release_answers_are_checked() {
    let pending = Ledger::default().reserve(1, 4);
    let settled = |status, charged| CreditReservation {
        status,
        charged,
        ..pending.clone()
    };
    assert!(checked_settlement(settled(ReservationStatus::Settled, Some(4)), 9).is_ok());
    assert!(checked_settlement(settled(ReservationStatus::Settled, Some(5)), 9).is_err());
    assert!(checked_settlement(settled(ReservationStatus::Settled, Some(3)), 2).is_err());
    assert!(checked_settlement(settled(ReservationStatus::Settled, None), 2).is_err());
    assert!(checked_settlement(settled(ReservationStatus::Pending, Some(1)), 2).is_err());
    assert!(checked_release(settled(ReservationStatus::Released, None)).is_ok());
    assert!(checked_release(settled(ReservationStatus::Released, Some(1))).is_err());
    assert!(checked_release(settled(ReservationStatus::Settled, Some(1))).is_err());
}
