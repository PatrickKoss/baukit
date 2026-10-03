use baukit_erasure::{ErasureFuture, IdentityAccountDeleter, IdentityDeletionError};
use std::{collections::VecDeque, sync::Mutex};

/// Scripted identity deletion fake. Unscripted calls succeed, including repeats.
#[derive(Default)]
pub struct FakeIdentityAccountDeleter {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    outcomes: VecDeque<Result<(), IdentityDeletionError>>,
    calls: Vec<String>,
}
impl FakeIdentityAccountDeleter {
    /// Adds the outcome for the next deletion attempt.
    pub fn push_outcome(&self, outcome: Result<(), IdentityDeletionError>) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .outcomes
            .push_back(outcome);
    }
    /// Returns attempted subjects in call order. Use only in tests, never logs.
    pub fn calls(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .calls
            .clone()
    }
}
impl IdentityAccountDeleter for FakeIdentityAccountDeleter {
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(async move {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.calls.push(subject.to_owned());
            state.outcomes.pop_front().unwrap_or(Ok(()))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn scripts_failures_and_records_repeated_deletion() {
        let fake = FakeIdentityAccountDeleter::default();
        fake.push_outcome(Err(IdentityDeletionError::Retryable));
        fake.push_outcome(Err(IdentityDeletionError::Permanent));
        assert_eq!(
            fake.delete_account("test").await,
            Err(IdentityDeletionError::Retryable)
        );
        assert_eq!(
            fake.delete_account("test").await,
            Err(IdentityDeletionError::Permanent)
        );
        assert_eq!(fake.delete_account("test").await, Ok(()));
        assert_eq!(fake.calls(), ["test", "test", "test"]);
    }
}
