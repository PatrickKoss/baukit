/** Receipt returned after the server accepts or completes product-profile erasure. */
export type ErasureReceipt =
  | { readonly operationId: string | null; readonly status: 'completed' }
  | { readonly operationId: string; readonly status: 'pending' };

/** Dependencies for the product-profile erasure sequence. */
export interface ProductProfileErasureDependencies {
  readonly beforeServerErase?: readonly (() => Promise<void>)[];
  readonly eraseServerProfile: () => Promise<ErasureReceipt>;
  readonly eraseLocalPartition: () => Promise<void>;
  readonly signOut: () => Promise<void>;
}

export type ProductProfileErasureStage = 'before-server' | 'server' | 'local' | 'sign-out';

/** Error detail safe to show in diagnostics without user or request content. */
export interface ProductProfileErasureIssue {
  readonly stage: ProductProfileErasureStage;
  readonly cause: string;
}

export type ProductProfileErasureResult =
  | {
      readonly status: 'erased';
      readonly receipt: ErasureReceipt;
      readonly warnings: readonly ProductProfileErasureIssue[];
    }
  | {
      readonly status: 'server-failure' | 'ambiguous';
      readonly error: ProductProfileErasureIssue;
      readonly warnings: readonly ProductProfileErasureIssue[];
    }
  | {
      readonly status: 'local-failure';
      readonly receipt: ErasureReceipt;
      readonly error: ProductProfileErasureIssue;
      readonly signOutError: ProductProfileErasureIssue | null;
      readonly warnings: readonly ProductProfileErasureIssue[];
    }
  | {
      readonly status: 'signout-failure';
      readonly receipt: ErasureReceipt;
      readonly error: ProductProfileErasureIssue;
      readonly warnings: readonly ProductProfileErasureIssue[];
    };

/** Marks a server error whose commit status cannot be determined safely. */
export class AmbiguousProductProfileErasureError extends Error {
  public override readonly name = 'AmbiguousProductProfileErasureError';
  public readonly code = 'product_profile_erasure_ambiguous' as const;

  public constructor(cause?: unknown) {
    super('The server erasure outcome is unknown.', cause === undefined ? undefined : { cause });
  }
}

const SAFE_CAUSE_NAMES = new Set([
  'AbortError',
  'AggregateError',
  'Error',
  'EvalError',
  'NetworkError',
  'RangeError',
  'ReferenceError',
  'SyntaxError',
  'TimeoutError',
  'TypeError',
  'URIError',
]);

function causeName(cause: unknown): string {
  if (!(cause instanceof Error)) return 'UnknownError';
  return SAFE_CAUSE_NAMES.has(cause.name) ? cause.name : 'Error';
}

function issue(stage: ProductProfileErasureStage, cause: unknown): ProductProfileErasureIssue {
  return { stage, cause: causeName(cause) };
}

function isAmbiguous(cause: unknown): boolean {
  return (
    cause instanceof AmbiguousProductProfileErasureError ||
    (typeof cause === 'object' &&
      cause !== null &&
      Reflect.get(cause, 'code') === 'product_profile_erasure_ambiguous')
  );
}

function ambiguousCause(cause: unknown): unknown {
  if (cause instanceof Error && cause.cause !== undefined) return cause.cause;
  return cause;
}

function validateErasureReceipt(receipt: unknown): asserts receipt is ErasureReceipt {
  if (typeof receipt !== 'object' || receipt === null) {
    throw new AmbiguousProductProfileErasureError(new TypeError('Invalid erasure receipt.'));
  }
  const operationId: unknown = Reflect.get(receipt, 'operationId');
  const status: unknown = Reflect.get(receipt, 'status');
  const hasOperationId = typeof operationId === 'string' && operationId.trim().length > 0;
  if (
    (status === 'completed' && (operationId === null || hasOperationId)) ||
    (status === 'pending' && hasOperationId)
  ) {
    return;
  }
  throw new AmbiguousProductProfileErasureError(new TypeError('Invalid erasure receipt.'));
}

/** Erases a product profile while preserving retryability at the server boundary. */
export async function eraseProductProfile(
  dependencies: ProductProfileErasureDependencies,
): Promise<ProductProfileErasureResult> {
  const warnings: ProductProfileErasureIssue[] = [];
  for (const hook of dependencies.beforeServerErase ?? []) {
    try {
      await hook();
    } catch (cause) {
      warnings.push(issue('before-server', cause));
    }
  }

  let receipt: ErasureReceipt;
  try {
    const candidate: unknown = await dependencies.eraseServerProfile();
    validateErasureReceipt(candidate);
    receipt = candidate;
  } catch (cause) {
    if (isAmbiguous(cause)) {
      return { status: 'ambiguous', error: issue('server', ambiguousCause(cause)), warnings };
    }
    return { status: 'server-failure', error: issue('server', cause), warnings };
  }

  let localError: ProductProfileErasureIssue | undefined;
  let signOutError: ProductProfileErasureIssue | undefined;
  try {
    await dependencies.eraseLocalPartition();
  } catch (cause) {
    localError = issue('local', cause);
  } finally {
    try {
      await dependencies.signOut();
    } catch (cause) {
      signOutError = issue('sign-out', cause);
    }
  }

  if (localError !== undefined) {
    return {
      status: 'local-failure',
      receipt,
      error: localError,
      signOutError: signOutError ?? null,
      warnings,
    };
  }
  if (signOutError !== undefined) {
    return { status: 'signout-failure', receipt, error: signOutError, warnings };
  }
  return { status: 'erased', receipt, warnings };
}
