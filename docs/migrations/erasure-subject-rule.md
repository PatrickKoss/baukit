# Erasure subjects must pass the subject rule

Since 0.31.0 the identity deletion worker rejects a subject that is empty, `.`, `..`, longer than 256
bytes or holds a control character (`baukit_erasure::kernel::is_well_formed_subject`). In 0.31.0
`ErasureService` still accepted and queued such a subject. The worker then failed the job, and a product
that had dropped the `erasure_job_failed` trigger kept the operation `pending` forever. Anything polling
for `completed` or `failed` hung.

## What changed

- `ErasureService::preflight`, `erase` and `fence_as_operator_in_transaction` check the same rule before
  they lock or write anything. A malformed subject returns the new `ErasureError::InvalidSubject`, and no
  operation, fence or job is created. An empty subject still returns `ErasureError::Configuration`.
  `InvalidSubject` renders as a 500, because the product builds the subject, not the client.
- When the worker gets a payload it cannot run, it sets the operation to `failed` with a failed receipt
  if the payload still has a readable `operationId`, logs the job id and operation id, and dead-letters
  the job. This no longer depends on the `erasure_job_failed` trigger. If the `operationId` is missing
  or not a UUID, the worker logs `identity deletion payload has no operation id` with the job id and
  dead-letters the job. No operation can be tied to that job, so none is updated.

## What to do

If your subject can contain a control character, change how you build it. A composite like
`format!("{issuer}\n{subject}")` must switch to a separator the rule allows, for example a space, `|` or
`#`. Keep it at 256 bytes or less. The subject is hashed with the erasure key, so the new format produces
new hashes. Erase or fence with the same format your session guards use, or old fences stop matching.

Match on `ErasureError::InvalidSubject` if you map erasure errors yourself.

## Jobs already queued

Jobs enqueued by 0.31.0 or earlier with a malformed subject are not retried. On the next attempt the
worker fails their operation, so the receipt reads `failed` and pollers stop waiting. The provider
account was not deleted. To finish those erasures, run `fence_as_operator_in_transaction` again with
the subject in its new format, which queues a fresh job.
