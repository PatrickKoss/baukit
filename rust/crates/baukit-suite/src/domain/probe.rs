use super::{DeliveryAction, DeliveryHealth, LinkStatus, SUITE_CIRCUIT_FAILURES, SuiteLink};
use crate::error_codes::SuiteErrorCode;
use jiff::Timestamp;
use std::time::Duration;

/// The first probe of a disabled link waits this long; each failed probe doubles it.
pub const SUITE_FIRST_PROBE_DELAY: Duration = Duration::from_secs(3_600);
/// The longest wait between probes of a disabled link.
pub const SUITE_MAX_PROBE_DELAY: Duration = Duration::from_secs(86_400);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryState {
    pub status: LinkStatus,
    pub health: DeliveryHealth,
    pub consecutive_failures: u32,
    pub next_probe_at: Option<Timestamp>,
    pub failed_probes: u32,
}

impl From<&SuiteLink> for DeliveryState {
    fn from(link: &SuiteLink) -> Self {
        Self {
            status: link.status,
            health: link.delivery_health,
            consecutive_failures: link.consecutive_failures,
            next_probe_at: link.next_probe_at,
            failed_probes: link.failed_probes,
        }
    }
}

/// The link state after a finished delivery attempt at `now`. A delivery,
/// including a probe, makes the link healthy. Each failed attempt counts toward
/// [`SUITE_CIRCUIT_FAILURES`]; reaching it disables the link and schedules the
/// first probe. Deferred attempts never reached the peer and change nothing.
pub fn after_delivery(
    state: DeliveryState,
    action: DeliveryAction,
    now: Timestamp,
) -> DeliveryState {
    match action {
        DeliveryAction::Delivered => DeliveryState {
            status: state.status,
            health: DeliveryHealth::Healthy,
            consecutive_failures: 0,
            next_probe_at: None,
            failed_probes: 0,
        },
        DeliveryAction::Revoked => DeliveryState {
            status: LinkStatus::Revoked,
            ..state
        },
        DeliveryAction::Unauthorized => failed(
            DeliveryState {
                status: LinkStatus::NeedsAttention,
                health: DeliveryHealth::NeedsAttention,
                ..state
            },
            now,
        ),
        DeliveryAction::Retry { .. } | DeliveryAction::Rejected => failed(state, now),
        DeliveryAction::LinkMissing
        | DeliveryAction::LinkRevoked
        | DeliveryAction::Disabled
        | DeliveryAction::Deferred { .. } => state,
    }
}

fn failed(state: DeliveryState, now: Timestamp) -> DeliveryState {
    if state.health == DeliveryHealth::Disabled {
        return state;
    }
    let consecutive_failures = state.consecutive_failures.saturating_add(1);
    if consecutive_failures >= SUITE_CIRCUIT_FAILURES {
        return DeliveryState {
            health: DeliveryHealth::Disabled,
            consecutive_failures,
            next_probe_at: Some(later(now, probe_delay(0))),
            failed_probes: 0,
            ..state
        };
    }
    DeliveryState {
        health: match state.health {
            DeliveryHealth::NeedsAttention => DeliveryHealth::NeedsAttention,
            _ => DeliveryHealth::Degraded,
        },
        consecutive_failures,
        ..state
    }
}

/// The wait before the probe after `failed_probes` failed ones: one hour,
/// doubling, at most [`SUITE_MAX_PROBE_DELAY`].
pub fn probe_delay(failed_probes: u32) -> Duration {
    2_u32
        .checked_pow(failed_probes)
        .and_then(|factor| SUITE_FIRST_PROBE_DELAY.checked_mul(factor))
        .map_or(SUITE_MAX_PROBE_DELAY, |delay| {
            delay.min(SUITE_MAX_PROBE_DELAY)
        })
}

/// The probe schedule after claiming a probe at `now`. The claim counts as a
/// failure until the probe is delivered, so a crash mid-probe cannot probe faster.
pub fn claim_probe(state: DeliveryState, now: Timestamp) -> DeliveryState {
    DeliveryState {
        next_probe_at: Some(later(
            now,
            probe_delay(state.failed_probes.saturating_add(1)),
        )),
        failed_probes: state.failed_probes.saturating_add(1),
        ..state
    }
}

fn later(now: Timestamp, delay: Duration) -> Timestamp {
    jiff::SignedDuration::try_from(delay)
        .ok()
        .and_then(|delay| now.checked_add(delay).ok())
        .unwrap_or(Timestamp::MAX)
}

impl DeliveryAction {
    /// The code stored as the link's last failure: the permanent code, or
    /// `suite_peer_unreachable` for a retry.
    pub const fn failure_code(self) -> Option<&'static str> {
        match self {
            Self::Retry { .. } => Some(SuiteErrorCode::SuitePeerUnreachable.as_str()),
            _ => self.permanent_code(),
        }
    }
}
