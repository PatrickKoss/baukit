export type DetailRouteState<T> =
  | { readonly status: 'loading' }
  | { readonly status: 'invalid' }
  | { readonly status: 'not-found' }
  | { readonly status: 'error'; readonly message: string }
  | { readonly status: 'ready'; readonly value: T };

interface DetailRouteInput<T> {
  readonly id: string | null;
  readonly isValidId: (id: string) => boolean;
  readonly loading: boolean;
  readonly error: unknown;
  readonly value: T | undefined;
}

export function deriveDetailRouteState<T>({
  id,
  isValidId,
  loading,
  error,
  value,
}: DetailRouteInput<T>): DetailRouteState<T> {
  if (id === null || !isValidId(id)) {
    return { status: 'invalid' };
  }
  if (value !== undefined) {
    return { status: 'ready', value };
  }
  if (error !== null && error !== undefined) {
    return {
      status: 'error',
      message: error instanceof Error ? error.message : 'The detail could not be loaded.',
    };
  }
  if (loading) {
    return { status: 'loading' };
  }
  return { status: 'not-found' };
}
