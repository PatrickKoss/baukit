use serde_json::json;
use tokio::sync::oneshot;

use super::*;

#[tokio::test]
async fn owned_operation_finishes_after_caller_cancellation() {
    let (started_sender, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (finished_sender, finished) = oneshot::channel();
    let caller = tokio::spawn(owned_operation(async move {
        started_sender.send(()).expect("caller waits for the start");
        released
            .await
            .expect("operation is released after cancellation");
        finished_sender.send(42).expect("completion is observed");
        Ok(())
    }));
    started.await.expect("operation starts");
    caller.abort();
    assert!(
        caller
            .await
            .expect_err("caller is cancelled")
            .is_cancelled()
    );
    release
        .send(())
        .expect("owned operation survives cancellation");
    assert_eq!(finished.await.expect("owned operation finishes"), 42);
}

#[test]
fn new_job_defaults_are_claimable_and_unique() {
    let job = NewJob::new("email.send", json!({"message_id": 7}), 3);
    assert_eq!(job.job_type, "email.send");
    assert_eq!(job.max_attempts, 3);
    assert!(job.idempotency_key.is_none());
    assert!(job.run_after >= job.created_at);
}

#[test]
fn active_deduplication_validates_keys_and_rejects_mixed_modes() {
    let job = NewJob::new("cache.refresh", json!({}), 3);
    for key in ["", " \t", &"x".repeat(MAX_DEDUPE_KEY_LENGTH + 1)] {
        assert!(matches!(
            validate_deduplicated_job(&job, key),
            Err(StoreError::InvalidInput(_))
        ));
    }
    assert!(validate_deduplicated_job(&job, &"é".repeat(MAX_DEDUPE_KEY_LENGTH)).is_ok());
    assert!(matches!(
        validate_deduplicated_job(&job.idempotency_key("permanent"), "active"),
        Err(StoreError::InvalidInput(_))
    ));
    assert!(validate_deduplicated_job(&NewJob::new(" ", json!({}), 3), "key").is_err());
    assert!(validate_deduplicated_job(&NewJob::new("cache.refresh", json!({}), 0), "key").is_err());
}

#[test]
fn claim_job_types_must_be_a_non_empty_set_of_valid_identifiers() {
    assert!(validate_job_types(&["email.send", "sms.send"]).is_ok());
    for invalid in [&[][..], &[" "][..], &["email.send", ""][..]] {
        assert!(matches!(
            validate_job_types(invalid),
            Err(StoreError::InvalidInput(_))
        ));
    }
    let too_long = "x".repeat(MAX_JOB_TYPE_LENGTH + 1);
    assert!(validate_job_types(&[too_long.as_str()]).is_err());
}

#[test]
fn error_text_is_bounded_on_character_boundaries() {
    let error = "é".repeat(MAX_ERROR_LENGTH + 1);
    assert_eq!(bound_error(&error).chars().count(), MAX_ERROR_LENGTH);
}
