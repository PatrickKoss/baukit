# Changelog

All notable changes to `baukit-resilience` are documented here.

## [Unreleased]

- The `task-deadline` feature adds `deadline::scope` and `deadline::current`, a tokio task-local that carries the current deadline.
- New crate: `Clock`, `Backoff`, `CircuitBreaker`, `BreakerSet`, `Deadline` and `StaleCache`.
