# Changelog

## [Unreleased]

- Group worker metrics by `job_kind` in shared dashboards, recording rules, and alerts. Test distinct handler types under one scrape job.

- Replace MinIO with digest-pinned RustFS 1.0.1 in the local platform. Use AWS CLI bucket bootstrap, keep `baukit-local` and `posthog`, and connect local Loki, PostHog, and PostgreSQL backups through path-style S3 in `us-east-1`.

- Refresh platform charts within their current majors and vendor Barman Cloud 0.15.1. Update PostHog Redis to 6.2.24, ZooKeeper to 3.9.5, Redpanda to 25.3.17, and BusyBox to 1.38. PostgreSQL stays on 18.6 and PostHog PostgreSQL on 14.1.

- Pin the CNPG operator default, primary and restore-test clusters to PostgreSQL
  `18.6-system-trixie`.
