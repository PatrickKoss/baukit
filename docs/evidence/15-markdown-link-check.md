# 15. Markdown link check

## Source product files

- `/home/patrick/projects/redemut/scripts/check-doc-links.mjs`
- `/home/patrick/projects/redemut/.github/workflows/ci.yml`
- `/home/patrick/projects/redemut/docs/baukit-playback-audit.md`

## Observed repeated glue

Redemut carries a dependency-free script and CI step to reject missing local inline and reference-link targets in project documentation.

## Baukit owner

The generated strict quality profile owns the local Markdown file-link gate. Products own the checked roots and documentation layout.

## Public types and errors

`scripts/check-markdown-links.py [ROOT ...]` checks committed Markdown. Exit code 0 means every local target exists, 1 reports missing targets as `SOURCE:LINE -> TARGET`, and 2 means committed files could not be listed.

## Product-owned inputs

Products own Markdown content and may replace the default `README.md`, `CLAUDE.md`, `AGENTS.md`, and `docs` roots in the gate command.

## Concurrency, failure, privacy, and cleanup

The check is read-only and concurrent runs do not share state. Invalid UTF-8 or a failed `git ls-files` call stops the check. It reads committed Markdown and prints link paths, not file contents. It creates no temporary files and makes no network requests.

## Supported runtimes

Python 3 and Git in generated strict projects, locally and on Linux CI.

## Product adoption change

Redemut can use the generated strict gate and delete `scripts/check-doc-links.mjs` plus its dedicated workflow command.
