/** A signal that aborts when the parent aborts or the timeout elapses. */
export interface AttemptSignal {
  readonly signal: AbortSignal;
  timedOut(): boolean;
  dispose(): void;
}

/**
 * Links the caller's signal with a per-attempt timeout.
 *
 * `AbortSignal.any` is missing on React Native, so the two sources are linked by hand.
 */
export function attemptSignal(parent: AbortSignal, timeoutMs: number | null): AttemptSignal {
  if (timeoutMs === null) {
    return { signal: parent, timedOut: () => false, dispose: () => undefined };
  }
  const controller = new AbortController();
  let expired = false;
  const onParentAbort = (): void => {
    controller.abort(parent.reason);
  };
  const timer = globalThis.setTimeout(() => {
    expired = true;
    controller.abort(new Error('API request timed out'));
  }, timeoutMs);
  if (parent.aborted) {
    onParentAbort();
  } else {
    parent.addEventListener('abort', onParentAbort, { once: true });
  }
  return {
    signal: controller.signal,
    timedOut: () => expired,
    dispose: () => {
      globalThis.clearTimeout(timer);
      parent.removeEventListener('abort', onParentAbort);
    },
  };
}
