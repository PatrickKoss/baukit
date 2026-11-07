import { OidcError, safeAuthErrorMessage } from '@baukit/auth-web';
import { describe, expect, it } from 'vitest';

import { sessionFailure } from './session-failure';

describe('sessionFailure', () => {
  it('offers a retry for a retryable sign-in error', () => {
    expect(sessionFailure(new OidcError('refresh_failed', { retryable: true }))).toEqual({
      kind: 'retry',
    });
  });

  it('reports a non-retryable sign-in error with its safe message', () => {
    const cause = new OidcError('refresh_failed', { status: 400 });
    expect(sessionFailure(cause)).toEqual({ kind: 'error', message: safeAuthErrorMessage(cause) });
  });

  it('reports anything else without leaking its message', () => {
    const failure = sessionFailure(new Error('secret upstream detail'));
    expect(failure).toEqual({ kind: 'error', message: safeAuthErrorMessage(new Error('x')) });
    expect(JSON.stringify(failure)).not.toContain('secret upstream detail');
  });
});
