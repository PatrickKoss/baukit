# Changelog

All notable changes to `baukit-resilience` are documented here.

## [Unreleased]

- The `startup` feature adds `startup::retry_within`, which retries transient errors within a startup budget with full jitter from 250 ms to 5 s, plus `StartupError` and `RetryAttempt`. Callers log retries from the `on_retry` callback, so the crate still does no I/O.
- The `task-deadline` feature adds `deadline::scope` and `deadline::current`, a tokio task-local that carries the current deadline.
- New crate: `Clock`, `Backoff`, `CircuitBreaker`, `BreakerSet`, `Deadline` and `StaleCache`.
