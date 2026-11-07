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

interface FocusScope {
  readonly runtime: RouteFocusRuntime;
  readonly timeoutMs: number;
}

function isReachable(document: Document, target: RouteFocusTarget): boolean {
  if (target.ownerDocument !== document || !target.isConnected) return false;
  if (target.closest(UNREACHABLE_ANCESTOR) !== null) return false;
  return !('disabled' in target && target.disabled === true);
}

function activeTarget(document: Document): RouteFocusTarget | null {
  return asRouteFocusTarget(document.activeElement);
}

/** True when the user moved focus to some other reachable element we must not override. */
function focusMovedElsewhere(
  document: Document,
  active: RouteFocusTarget | null,
  allowed: ReadonlySet<RouteFocusTarget>,
): boolean {
  return (
    active !== null &&
    active !== document.body &&
    isReachable(document, active) &&
    !allowed.has(active)
  );
}

type AttemptOutcome = 'done' | 'retry';

function attemptFocus(
  document: Document,
  target: RouteFocusTarget | null,
  allowed: ReadonlySet<RouteFocusTarget>,
): AttemptOutcome {
  const active = activeTarget(document);
  if (active === target && target !== null && isReachable(document, target)) return 'done';
  if (focusMovedElsewhere(document, active, allowed)) return 'done';
  if (target === null || !isReachable(document, target)) return 'retry';
  target.focus({ preventScroll: true });
  const focused = activeTarget(document);
  return focused === target || focusMovedElsewhere(document, focused, allowed) ? 'done' : 'retry';
}

function retryFocus(
  scope: FocusScope,
  getTarget: () => RouteFocusTarget | null,
  allowedActiveTargets: ReadonlySet<RouteFocusTarget>,
  observedTargets?: Set<RouteFocusTarget>,
): () => void {
  const { runtime, timeoutMs } = scope;
  const startedAt = runtime.now();
  let frame: number | null = null;
  let timer: number | null = null;
  let stopped = false;

  const stop = (): void => {
    if (stopped) return;
    stopped = true;
    if (frame !== null) runtime.cancelAnimationFrame(frame);
    if (timer !== null) runtime.clearTimeout(timer);
    frame = null;
    timer = null;
  };

  const attempt = (): void => {
    frame = null;
    if (stopped) return;
    if (runtime.now() - startedAt >= timeoutMs) {
      stop();
      return;
    }
    const target = getTarget();
    if (target !== null) observedTargets?.add(target);
    if (attemptFocus(runtime.document, target, allowedActiveTargets) === 'done') {
      stop();
      return;
    }
    frame = runtime.requestAnimationFrame(attempt);
  };

  timer = runtime.setTimeout(stop, timeoutMs);
  frame = runtime.requestAnimationFrame(attempt);
  return stop;
}

/** Tracks route entry and restores focus when the returned cleanup runs. */
export function createRouteFocusController(
  runtime: RouteFocusRuntime = browserRuntime(),
  options: RouteFocusControllerOptions = {},
): RouteFocusController {
  const { document } = runtime;
  const scope: FocusScope = {
    runtime,
    timeoutMs: Math.max(0, options.focusRetryTimeoutMs ?? DEFAULT_FOCUS_RETRY_TIMEOUT_MS),
  };
  let lastFocusedTarget: RouteFocusTarget | null = null;
  let cancelRetry: () => void = () => undefined;
  let disposed = false;
  let session = 0;

  const onFocusIn = (event: FocusEvent): void => {
    const target = asRouteFocusTarget(event.target);
    if (target !== null && target !== document.body && isReachable(document, target)) {
      lastFocusedTarget = target;
    }
  };
  document.addEventListener('focusin', onFocusIn, true);

  const enterRoute = (getTarget: () => RouteFocusTarget | null): (() => void) => {
    cancelRetry();
    const id = ++session;
    const active = activeTarget(document);
    const returnTarget =
      active !== null && active !== document.body && isReachable(document, active)
        ? active
        : lastFocusedTarget;
    const allowedOnEntry = new Set<RouteFocusTarget>();
    if (returnTarget !== null) allowedOnEntry.add(returnTarget);
    const entryTargets = new Set<RouteFocusTarget>();
    cancelRetry = retryFocus(scope, getTarget, allowedOnEntry, entryTargets);

    return () => {
      if (disposed || id !== session) return;
      session += 1;
      cancelRetry();
      if (returnTarget === null) return;
      cancelRetry = retryFocus(scope, () => returnTarget, entryTargets);
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
