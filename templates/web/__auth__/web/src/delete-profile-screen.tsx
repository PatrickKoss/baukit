import { useEffect, useRef, useState } from 'react';
import { announce } from '@baukit/a11y-core/web';
import type { ProductProfileErasureResult } from '@baukit/data-contracts';
import { ApiError } from '@baukit/api-runtime';
import type { ProfileErasureOperation } from '@baukit/api-runtime/erasure';

import { deleteProfileCopy } from './delete-profile-copy';

interface DeleteProfileScreenProps {
  readonly erase: () => Promise<ProductProfileErasureResult>;
  readonly poll?: (operationId: string, signal: AbortSignal) => Promise<ProfileErasureOperation>;
  readonly language?: string;
  readonly available?: boolean;
}

type State =
  'idle' | 'confirming' | 'erasing' | 'preparation-failure' | ProductProfileErasureResult;

export function DeleteProfileScreen({
  erase,
  poll,
  available = true,
  language = navigator.language,
}: DeleteProfileScreenProps) {
  const copy = deleteProfileCopy(language);
  const [state, setState] = useState<State>('idle');
  const [operation, setOperation] = useState<ProfileErasureOperation>();
  const [statusError, setStatusError] = useState<'unavailable' | 'expired' | null>(null);
  const [pollRound, setPollRound] = useState(0);
  const [finishedRound, setFinishedRound] = useState<number | null>(null);
  const running = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const status = useRef<HTMLParagraphElement>(null);
  const result = typeof state === 'string' ? undefined : state;
  const messages = {
    erased: copy.erased,
    pending: copy.pending,
    ambiguous: copy.ambiguous,
    'server-failure': copy.serverFailure,
    'local-failure': copy.localFailure,
    'signout-failure': copy.signoutFailure,
  };
  const resultMessage = result === undefined ? undefined : messages[result.status];
  let message = state === 'erasing' ? copy.busy : resultMessage;
  if (state === 'preparation-failure') message = copy.preparationFailure;
  if (statusError === 'unavailable') message = copy.statusError;
  if (statusError === 'expired') message = copy.statusExpired;
  if (operation?.status === 'completed') message = copy.erased;
  if (operation?.status === 'failed') message = copy.failed;
  const retryable =
    state === 'preparation-failure' ||
    result?.status === 'server-failure' ||
    result?.status === 'ambiguous';
  const pendingId = result?.status === 'pending' ? result.receipt.operationId : null;
  const checking = poll !== undefined && pendingId !== null && finishedRound !== pollRound;
  const canCheck =
    statusError !== 'expired' &&
    poll !== undefined &&
    pendingId !== null &&
    (operation === undefined || operation.status === 'pending');

  useEffect(() => {
    if (state === 'confirming') cancel.current?.focus();
    else if (message === undefined) heading.current?.focus();
    else {
      status.current?.focus();
      announce(message);
    }
  }, [message, state]);

  useEffect(() => {
    if (poll === undefined || pendingId === null) return;
    const readStatus = poll;
    const operationId = pendingId;
    const controller = new AbortController();
    async function checkStatus(): Promise<void> {
      try {
        const next = await readStatus(operationId, controller.signal);
        if (!controller.signal.aborted) setOperation(next);
      } catch (cause) {
        if (!controller.signal.aborted) {
          setStatusError(
            cause instanceof ApiError && cause.status === 401 ? 'expired' : 'unavailable',
          );
        }
      } finally {
        if (!controller.signal.aborted) setFinishedRound(pollRound);
      }
    }
    void checkStatus();
    return () => {
      controller.abort();
    };
  }, [pendingId, poll, pollRound]);

  async function submit(): Promise<void> {
    if (running.current) return;
    running.current = true;
    setState('erasing');
    try {
      setState(await erase());
    } catch {
      setState('preparation-failure');
    } finally {
      running.current = false;
    }
  }

  return (
    <section
      className="panel"
      aria-labelledby="delete-profile-title"
      aria-busy={state === 'erasing' || checking}
    >
      <h2 id="delete-profile-title" ref={heading} tabIndex={-1}>
        {copy.title}
      </h2>
      <p>{state === 'confirming' ? copy.confirmation : copy.description}</p>
      {message === undefined ? null : (
        <p ref={status} tabIndex={-1} role="status">
          {message}
        </p>
      )}
      {state === 'idle' ? (
        <button
          className="action secondary"
          type="button"
          disabled={!available}
          onClick={() => {
            setState('confirming');
          }}
        >
          {copy.continue}
        </button>
      ) : null}
      {state === 'confirming' ? (
        <div className="actions">
          <button
            ref={cancel}
            className="action secondary"
            type="button"
            onClick={() => {
              setState('idle');
            }}
          >
            {copy.cancel}
          </button>
          <button
            className="action"
            type="button"
            onClick={() => {
              void submit();
            }}
          >
            {copy.confirm}
          </button>
        </div>
      ) : null}
      {retryable ? (
        <button
          className="action"
          type="button"
          onClick={() => {
            void submit();
          }}
        >
          {copy.retry}
        </button>
      ) : null}
      {canCheck ? (
        <button
          className="action secondary"
          type="button"
          disabled={checking}
          onClick={() => {
            setStatusError(null);
            setPollRound((round) => round + 1);
          }}
        >
          {copy.checkStatus}
        </button>
      ) : null}
    </section>
  );
}
