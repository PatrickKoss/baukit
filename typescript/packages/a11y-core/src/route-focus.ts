const DEFAULT_FOCUS_RETRY_TIMEOUT_MS = 1500;
const UNREACHABLE_ANCESTOR = '[inert], [aria-hidden="true"], [hidden]';

export type RouteFocusTarget = Element & {
  focus(options?: FocusOptions): void;
};

export interface RouteFocusRuntime {
  document: Document;
  requestAnimationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
  setTimeout(callback: () => void, delayMs: number): number;
  clearTimeout(handle: number): void;
  now(): number;
}

export interface RouteFocusControllerOptions {
  focusRetryTimeoutMs?: number;
}

export interface RouteFocusController {
  enterRoute(target: () => RouteFocusTarget | null): () => void;
  dispose(): void;
}

function browserRuntime(): RouteFocusRuntime {
  return {
    document: globalThis.document,
    requestAnimationFrame: (callback) => globalThis.requestAnimationFrame(callback),
    cancelAnimationFrame: (handle) => {
      globalThis.cancelAnimationFrame(handle);
    },
    setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
    clearTimeout: (handle) => {
      globalThis.clearTimeout(handle);
    },
    now: () => Date.now(),
  };
}

function asRouteFocusTarget(value: unknown): RouteFocusTarget | null {
  if (typeof value !== 'object' || value === null) return null;
  const candidate = value as Partial<RouteFocusTarget>;
  return typeof candidate.focus === 'function' && typeof candidate.closest === 'function'
    ? (candidate as RouteFocusTarget)
    : null;
}

/** Tracks route entry and restores focus when the returned cleanup runs. */
export function createRouteFocusController(
  runtime: RouteFocusRuntime = browserRuntime(),
  options: RouteFocusControllerOptions = {},
): RouteFocusController {
  const { document } = runtime;
  const focusRetryTimeoutMs = Math.max(
    0,
    options.focusRetryTimeoutMs ?? DEFAULT_FOCUS_RETRY_TIMEOUT_MS,
  );
  let lastFocusedTarget: RouteFocusTarget | null = null;
  let cancelRetry: () => void = () => undefined;
  let disposed = false;
  let session = 0;

  const reachable = (target: RouteFocusTarget): boolean => {
    if (target.ownerDocument !== document || !target.isConnected) return false;
    if (target.closest(UNREACHABLE_ANCESTOR) !== null) return false;
    return !('disabled' in target && target.disabled === true);
  };

  const activeTarget = (): RouteFocusTarget | null => asRouteFocusTarget(document.activeElement);

  const onFocusIn = (event: FocusEvent) => {
    const target = asRouteFocusTarget(event.target);
    if (target !== null && target !== document.body && reachable(target)) {
      lastFocusedTarget = target;
    }
  };
  document.addEventListener('focusin', onFocusIn, true);

  const retryFocus = (
    getTarget: () => RouteFocusTarget | null,
    allowedActiveTargets: ReadonlySet<RouteFocusTarget>,
    observedTargets?: Set<RouteFocusTarget>,
  ): (() => void) => {
    const startedAt = runtime.now();
    let frame: number | null = null;
    let timer: number | null = null;
    let stopped = false;

    const stop = () => {
      if (stopped) return;
      stopped = true;
      if (frame !== null) runtime.cancelAnimationFrame(frame);
      if (timer !== null) runtime.clearTimeout(timer);
      frame = null;
      timer = null;
    };

    const attempt = () => {
      frame = null;
      if (stopped) return;
      if (runtime.now() - startedAt >= focusRetryTimeoutMs) {
        stop();
        return;
      }

      const target = getTarget();
      if (target !== null) observedTargets?.add(target);
      const active = activeTarget();
      if (active === target && target !== null && reachable(target)) {
        stop();
        return;
      }
      if (
        active !== null &&
        active !== document.body &&
        reachable(active) &&
        !allowedActiveTargets.has(active)
      ) {
        stop();
        return;
      }

      if (target !== null && reachable(target)) {
        target.focus({ preventScroll: true });
        const focused = activeTarget();
        if (
          focused === target ||
          (focused !== null &&
            focused !== document.body &&
            reachable(focused) &&
            !allowedActiveTargets.has(focused))
        ) {
          stop();
          return;
        }
      }

      frame = runtime.requestAnimationFrame(attempt);
    };

    timer = runtime.setTimeout(stop, focusRetryTimeoutMs);
    frame = runtime.requestAnimationFrame(attempt);
    return stop;
  };

  const enterRoute = (getTarget: () => RouteFocusTarget | null): (() => void) => {
    cancelRetry();
    const id = ++session;
    const active = activeTarget();
    const returnTarget =
      active !== null && active !== document.body && reachable(active) ? active : lastFocusedTarget;
    const allowedOnEntry = new Set<RouteFocusTarget>();
    if (returnTarget !== null) allowedOnEntry.add(returnTarget);
    const entryTargets = new Set<RouteFocusTarget>();
    cancelRetry = retryFocus(getTarget, allowedOnEntry, entryTargets);

    return () => {
      if (disposed || id !== session) return;
      session += 1;
      cancelRetry();
      if (returnTarget === null) return;
      cancelRetry = retryFocus(() => returnTarget, entryTargets);
    };
  };

  return {
    enterRoute,
    dispose() {
      if (disposed) return;
      disposed = true;
      session += 1;
      cancelRetry();
      document.removeEventListener('focusin', onFocusIn, true);
    },
  };
}
