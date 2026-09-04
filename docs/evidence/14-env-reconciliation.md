# 14. Environment reconciliation

## Source product files

- `/home/patrick/projects/eigenruhe/scripts/reconcile_env.py`
- `/home/patrick/projects/eigenruhe/scripts/test_reconcile_env.py`
- `/home/patrick/projects/eigenruhe/Makefile`
- `/home/patrick/projects/eigenruhe/README.md`

## Observed repeated glue

Eigenruhe appends new example assignments during setup because copying `.env.example` would replace local secrets and choices. Its tests cover creation, preservation, output, and idempotence.

## Baukit owner

The common generated-project scripts own environment-file reconciliation. Products own variable names and example values.

## Public types and errors

`scripts/reconcile-env.py EXAMPLE ENV` appends missing assignments and prints `added KEY`. Missing or unreadable paths return a nonzero Python error. `scripts/setup.sh` applies it to root, web, and mobile example files that exist.

## Product-owned inputs

Products own every `.env.example`, `.env`, value, comment, and setup invocation.

## Concurrency, failure, privacy, and cleanup

Setup is a single-writer operation and does not lock against concurrent runs. A read or write failure stops the command. Output contains key names, never values. The script creates no temporary or backup files. Existing bytes remain unchanged and repeated runs append nothing.

## Supported runtimes

Python 3 and POSIX shell on generated backend, web, mobile, and combined projects.

## Product adoption change

Eigenruhe can replace its Makefile reconciliation call with the generated setup command, then delete `scripts/reconcile_env.py` and `scripts/test_reconcile_env.py`.
