import { useEffect, useRef, useState, type ReactElement, type RefObject } from 'react';
import { announce } from '@baukit/a11y-core/web';
import type { ProductProfileErasureResult } from '@baukit/data-contracts';
import { ApiError } from '@baukit/api-runtime';
import type { ProfileErasureOperation } from '@baukit/api-runtime/erasure';

import { deleteProfileCopy, type ErasureCopy } from './delete-profile-copy';

type PollStatus = (operationId: string, signal: AbortSignal) => Promise<ProfileErasureOperation>;

interface DeleteProfileScreenProps {
  readonly erase: () => Promise<ProductProfileErasureResult>;
  readonly poll?: PollStatus;
  readonly language?: string;
  readonly available?: boolean;
}

type State =
  'idle' | 'confirming' | 'erasing' | 'preparation-failure' | ProductProfileErasureResult;
type StatusError = 'unavailable' | 'expired' | null;

const HTTP_UNAUTHORIZED = 401;

function resultMessage(copy: ErasureCopy, state: State): string | undefined {
  if (state === 'erasing') return copy.busy;
  if (state === 'preparation-failure') return copy.preparationFailure;
  if (typeof state === 'string') return undefined;
  const messages = {
    erased: copy.erased,
    pending: copy.pending,
    ambiguous: copy.ambiguous,
    'server-failure': copy.serverFailure,
    'local-failure': copy.localFailure,
    'signout-failure': copy.signoutFailure,
  };
  return messages[state.status];
}

function screenMessage(
  copy: ErasureCopy,
  state: State,
  statusError: StatusError,
  operation: ProfileErasureOperation | undefined,
): string | undefined {
  if (operation?.status === 'failed') return copy.failed;
  if (operation?.status === 'completed') return copy.erased;
  if (statusError === 'expired') return copy.statusExpired;
  if (statusError === 'unavailable') return copy.statusError;
  return resultMessage(copy, state);
}

function isRetryable(state: State): boolean {
  if (state === 'preparation-failure') return true;
  return typeof state !== 'string' && (state.status === 'server-failure' || state.status === 'ambiguous');
}

function pendingOperationId(state: State): string | null {
  return typeof state !== 'string' && state.status === 'pending' ? state.receipt.operationId : null;
}

/** Polls a pending erasure once per round; a new round starts a new request. */
function useOperationStatus(poll: PollStatus | undefined, pendingId: string | null) {
  const [operation, setOperation] = useState<ProfileErasureOperation>();
  const [statusError, setStatusError] = useState<StatusError>(null);
  const [pollRound, setPollRound] = useState(0);
  const [finishedRound, setFinishedRound] = useState<number | null>(null);

  useEffect(() => {
    if (poll === undefined || pendingId === null) return;
    const controller = new AbortController();
    async function checkStatus(readStatus: PollStatus, operationId: string): Promise<void> {
      try {
        const next = await readStatus(operationId, controller.signal);
        if (!controller.signal.aborted) setOperation(next);
      } catch (cause) {
        if (!controller.signal.aborted) {
          setStatusError(
            cause instanceof ApiError && cause.status === HTTP_UNAUTHORIZED ? 'expired' : 'unavailable',
          );
        }
      } finally {
        if (!controller.signal.aborted) setFinishedRound(pollRound);
      }
    }
    void checkStatus(poll, pendingId);
    return () => {
      controller.abort();
    };
  }, [pendingId, poll, pollRound]);

  const active = poll !== undefined && pendingId !== null;
  return {
    operation,
    statusError,
    checking: active && finishedRound !== pollRound,
    canCheck:
      active &&
      statusError !== 'expired' &&
      (operation === undefined || operation.status === 'pending'),
    checkAgain: (): void => {
      setStatusError(null);
      setPollRound((round) => round + 1);
    },
  };
}

function useErasure(erase: () => Promise<ProductProfileErasureResult>) {
  const [state, setState] = useState<State>('idle');
  const running = useRef(false);
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
  return { state, setState, submit };
}

interface FocusTargets {
  readonly heading: RefObject<HTMLHeadingElement | null>;
  readonly cancel: RefObject<HTMLButtonElement | null>;
  readonly status: RefObject<HTMLParagraphElement | null>;
}

function useScreenFocus(state: State, message: string | undefined, targets: FocusTargets): void {
  useEffect(() => {
    if (state === 'confirming') targets.cancel.current?.focus();
    else if (message === undefined) targets.heading.current?.focus();
    else {
      targets.status.current?.focus();
      announce(message);
    }
  }, [message, state, targets]);
}

function ConfirmActions({
  copy,
  cancelRef,
  onCancel,
  onConfirm,
}: {
  readonly copy: ErasureCopy;
  readonly cancelRef: RefObject<HTMLButtonElement | null>;
  readonly onCancel: () => void;
  readonly onConfirm: () => void;
}): ReactElement {
  return (
    <div className="actions">
      <button ref={cancelRef} className="action secondary" type="button" onClick={onCancel}>
        {copy.cancel}
      </button>
      <button className="action" type="button" onClick={onConfirm}>
        {copy.confirm}
      </button>
    </div>
  );
}

export function DeleteProfileScreen({
  erase,
  poll,
  available = true,
  language = navigator.language,
}: DeleteProfileScreenProps): ReactElement {
  const copy = deleteProfileCopy(language);
  const { state, setState, submit } = useErasure(erase);
  const status = useOperationStatus(poll, pendingOperationId(state));
  const heading = useRef<HTMLHeadingElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const statusLine = useRef<HTMLParagraphElement>(null);
  const [targets] = useState<FocusTargets>(() => ({ heading, cancel, status: statusLine }));
  const message = screenMessage(copy, state, status.statusError, status.operation);
  useScreenFocus(state, message, targets);
  const run = (): void => {
    void submit();
  };

  return (
    <section
      className="panel"
      aria-labelledby="delete-profile-title"
      aria-busy={state === 'erasing' || status.checking}
    >
      <h2 id="delete-profile-title" ref={heading} tabIndex={-1}>
        {copy.title}
      </h2>
      <p>{state === 'confirming' ? copy.confirmation : copy.description}</p>
      {message === undefined ? null : (
        <p ref={statusLine} tabIndex={-1} role="status">
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
        <ConfirmActions
          copy={copy}
          cancelRef={cancel}
          onCancel={() => {
            setState('idle');
          }}
          onConfirm={run}
        />
      ) : null}
      {isRetryable(state) ? (
        <button className="action" type="button" onClick={run}>
          {copy.retry}
        </button>
      ) : null}
      {status.canCheck ? (
        <button
          className="action secondary"
          type="button"
          disabled={status.checking}
          onClick={status.checkAgain}
        >
          {copy.checkStatus}
        </button>
      ) : null}
    </section>
  );
}
