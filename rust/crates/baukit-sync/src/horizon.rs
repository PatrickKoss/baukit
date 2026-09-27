//! Pull-cursor rules for tombstone purge horizons.
//!
//! A server that purges tombstones keeps one horizon per owner: the greatest
//! revision of any tombstone it purged. A pull cursor below that horizon can
//! no longer replay every deletion, so the client must rebuild from cursor
//! zero. The constants here are the stable wire signal that
//! `@baukit/sync-client` already decodes: HTTP 409 with error code
//! `resync_required` and the horizon in `details.horizon_revision`.

use thiserror::Error;

/// HTTP status of a stale-cursor response.
pub const RESYNC_REQUIRED_STATUS: u16 = 409;

/// Stable error `code` of a stale-cursor response.
pub const RESYNC_REQUIRED_CODE: &str = "resync_required";

/// Error `details` key that carries the owner's purge horizon.
pub const HORIZON_REVISION_DETAIL: &str = "horizon_revision";

/// Cursor that requests a full rebuild. It is valid at every horizon.
pub const FULL_RESYNC_CURSOR: i64 = 0;

/// Reason a pull cursor cannot be served incrementally.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum PullCursorError {
    /// The cursor is below zero, which no revision counter can produce.
    #[error("pull cursor must not be negative")]
    Negative,
    /// The cursor is above zero and below the owner's purge horizon.
    #[error("pull cursor is below the purge horizon {horizon_revision}")]
    ResyncRequired {
        /// The owner's current purge horizon.
        horizon_revision: i64,
    },
}

/// Checks a pull cursor against the owner's purge horizon.
///
/// Cursor zero is always accepted. A cursor equal to or above the horizon is a
/// valid incremental request. A cursor between zero and the horizon returns
/// [`PullCursorError::ResyncRequired`]. Pass zero as the horizon for an owner
/// that has none.
///
/// ```
/// use baukit_sync::horizon::{PullCursorError, check_pull_cursor};
///
/// assert_eq!(check_pull_cursor(0, 42), Ok(()));
/// assert_eq!(check_pull_cursor(42, 42), Ok(()));
/// assert_eq!(
///     check_pull_cursor(41, 42),
///     Err(PullCursorError::ResyncRequired { horizon_revision: 42 })
/// );
/// ```
///
/// # Errors
///
/// Returns [`PullCursorError::Negative`] for a negative cursor and
/// [`PullCursorError::ResyncRequired`] for a stale one.
pub const fn check_pull_cursor(cursor: i64, horizon_revision: i64) -> Result<(), PullCursorError> {
    if cursor < FULL_RESYNC_CURSOR {
        return Err(PullCursorError::Negative);
    }
    if cursor > FULL_RESYNC_CURSOR && cursor < horizon_revision {
        return Err(PullCursorError::ResyncRequired { horizon_revision });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_zero_is_accepted_at_every_horizon() {
        assert_eq!(check_pull_cursor(0, 0), Ok(()));
        assert_eq!(check_pull_cursor(0, i64::MAX), Ok(()));
    }

    #[test]
    fn cursor_at_or_above_the_horizon_is_incremental() {
        assert_eq!(check_pull_cursor(10, 10), Ok(()));
        assert_eq!(check_pull_cursor(11, 10), Ok(()));
        assert_eq!(check_pull_cursor(1, 0), Ok(()));
    }

    #[test]
    fn cursor_below_the_horizon_requires_resync() {
        assert_eq!(
            check_pull_cursor(9, 10),
            Err(PullCursorError::ResyncRequired {
                horizon_revision: 10
            })
        );
        assert_eq!(
            check_pull_cursor(1, 2),
            Err(PullCursorError::ResyncRequired {
                horizon_revision: 2
            })
        );
    }

    #[test]
    fn negative_cursor_is_rejected() {
        assert_eq!(check_pull_cursor(-1, 0), Err(PullCursorError::Negative));
    }
}
