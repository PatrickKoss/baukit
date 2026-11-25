//! Dependency-free decision functions shared by the adapters, tests and proofs.

use crate::IdentityDeletionError;

const NOT_FOUND: u16 = 404;
const TOO_MANY_REQUESTS: u16 = 429;
const SUCCESS: std::ops::RangeInclusive<u16> = 200..=299;
const SERVER_ERROR: std::ops::RangeInclusive<u16> = 500..=599;
const MAX_SUBJECT_BYTES: usize = 256;

/// Classifies an identity provider's answer to a user deletion.
///
/// Only a `2xx`, or a `404` whose provider error body names a missing user, counts as
/// deleted. A bare `404` usually means a wrong base URL, so it is a permanent error.
///
/// # Errors
///
/// [`IdentityDeletionError::Retryable`] for `429` and `5xx`, and
/// [`IdentityDeletionError::Permanent`] for every other status.
pub const fn deletion_status_outcome(
    status: u16,
    body_says_missing: bool,
) -> Result<(), IdentityDeletionError> {
    if status >= *SUCCESS.start() && status <= *SUCCESS.end() {
        return Ok(());
    }
    if status == NOT_FOUND && body_says_missing {
        return Ok(());
    }
    if status == TOO_MANY_REQUESTS
        || (status >= *SERVER_ERROR.start() && status <= *SERVER_ERROR.end())
    {
        return Err(IdentityDeletionError::Retryable);
    }
    Err(IdentityDeletionError::Permanent)
}

/// Returns whether `subject` can name exactly one provider user path segment.
pub fn is_well_formed_subject(subject: &str) -> bool {
    !subject.is_empty()
        && subject.len() <= MAX_SUBJECT_BYTES
        && !matches!(subject, "." | "..")
        && !subject.chars().any(char::is_control)
}

/// The fence state of one subject, in the same units as the session time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceRow {
    /// When the latest erasure fenced the subject.
    pub erased_at: i64,
    /// Whether any erasure operation for the subject is not completed.
    pub unfinished_operation: bool,
}

/// Mirror of the session fence query. A session is rejected when a fence exists and either
/// the session has no login time, logged in no later than the fence, or an erasure for the
/// subject is still pending or failed.
pub const fn fence_rejects(fence: Option<FenceRow>, authenticated_at: Option<i64>) -> bool {
    let Some(fence) = fence else {
        return false;
    };
    match authenticated_at {
        None => true,
        Some(at) => at <= fence.erased_at || fence.unfinished_operation,
    }
}

/// How an erasure request relates to an existing receipt for its idempotency key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayDecision {
    /// The key is unused. Proceed to the fence check.
    Fresh,
    /// The key belongs to this subject. Return its stored receipt.
    Replay,
    /// The key belongs to another subject.
    Conflict,
}

/// Decides replay from whether a receipt exists and whether its subject hash matches.
pub const fn replay_decision(previous_exists: bool, same_subject: bool) -> ReplayDecision {
    if !previous_exists {
        ReplayDecision::Fresh
    } else if same_subject {
        ReplayDecision::Replay
    } else {
        ReplayDecision::Conflict
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// K-23: every `u16` status, both body answers. No loops, so no unwind bound.
    #[kani::proof]
    fn deletion_outcome_ok_only_for_2xx_or_missing_404() {
        let status: u16 = kani::any();
        let missing: bool = kani::any();
        let outcome = deletion_status_outcome(status, missing);
        let deleted = (200..=299).contains(&status) || (status == 404 && missing);
        assert_eq!(outcome.is_ok(), deleted);
        if status == 404 && !missing {
            assert_eq!(outcome, Err(IdentityDeletionError::Permanent));
        }
    }

    /// K-22: every fence row and session time over the full `i64` range. No loops.
    #[kani::proof]
    fn fence_matches_truth_table() {
        let present: bool = kani::any();
        let fence = present.then(|| FenceRow {
            erased_at: kani::any(),
            unfinished_operation: kani::any(),
        });
        let has_login: bool = kani::any();
        let login: i64 = kani::any();
        let authenticated_at = has_login.then_some(login);
        let rejected = fence_rejects(fence, authenticated_at);
        match (fence, authenticated_at) {
            (None, _) => assert!(!rejected),
            (Some(_), None) => assert!(rejected),
            (Some(row), Some(at)) if row.unfinished_operation => {
                let _ = at;
                assert!(rejected);
            }
            (Some(row), Some(at)) => assert_eq!(rejected, at <= row.erased_at),
        }
    }

    /// K-22: all four replay inputs. No loops.
    #[kani::proof]
    fn replay_matches_truth_table() {
        let exists: bool = kani::any();
        let same: bool = kani::any();
        let decision = replay_decision(exists, same);
        assert_eq!(decision == ReplayDecision::Fresh, !exists);
        assert_eq!(decision == ReplayDecision::Conflict, exists && !same);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_not_found_is_permanent() {
        assert_eq!(deletion_status_outcome(204, false), Ok(()));
        assert_eq!(deletion_status_outcome(404, true), Ok(()));
        assert_eq!(
            deletion_status_outcome(404, false),
            Err(IdentityDeletionError::Permanent)
        );
        assert_eq!(
            deletion_status_outcome(503, true),
            Err(IdentityDeletionError::Retryable)
        );
    }

    #[test]
    fn subjects_are_single_visible_segments() {
        assert!(is_well_formed_subject("user_123"));
        for invalid in ["", ".", "..", "a\nb", &"x".repeat(MAX_SUBJECT_BYTES + 1)] {
            assert!(!is_well_formed_subject(invalid), "{invalid:?}");
        }
    }
}
