import { describe, expect, it } from 'vitest';

import {
  connectionStateFromServer,
  reduceConnectionState,
  type ConnectionState,
} from './connection-health.js';

describe('connection health', () => {
  it.each([
    ['healthy', 'connected', ['disconnect']],
    ['degraded', 'error', ['retry']],
    ['needs_reconnect', 'needs_reconnect', ['reconnect']],
    ['failed', 'error', ['retry']],
    ['revoked', 'needs_reconnect', ['reconnect']],
    ['pending_revocation', 'revocation_pending', ['retry']],
    ['disconnected', 'disconnected', ['connect']],
  ] as const)('maps server state %s to %s', (serverState, status, actions) => {
    const state = connectionStateFromServer({ state: serverState });
    expect(state.status).toBe(status);
    expect(state.availableActions).toEqual(actions);
  });

  it('never copies provider diagnostics into the user-facing state', () => {
    const rawProviderError = 'invalid_grant: token=provider-secret';
    const state = connectionStateFromServer({
      state: 'degraded',
      diagnosticCode: 'provider_unavailable',
      providerDiagnostic: rawProviderError,
    });

    expect(state.diagnostic).toEqual({ code: 'provider_unavailable', source: 'server' });
    expect(JSON.stringify(state)).not.toContain(rawProviderError);
  });

  it.each([
    'provider failed with account 42',
    'provider-failed',
    'UPSTREAM_FAILURE',
    `a${'b'.repeat(128)}`,
  ])('drops unsafe diagnostic code %s', (diagnosticCode) => {
    const state = connectionStateFromServer({
      state: 'failed',
      diagnosticCode,
      providerDiagnostic: diagnosticCode,
    });
    expect(state.diagnostic).toBeUndefined();
    expect(JSON.stringify(state)).not.toContain(diagnosticCode);
  });

  it('keeps reconnect intent when authorization is cancelled', () => {
    const reconnectable = connectionStateFromServer({ state: 'needs_reconnect' });
    const connecting = reduceConnectionState(reconnectable, { type: 'connect_requested' });
    expect(connecting).toMatchObject({ status: 'connecting', operation: 'reconnect' });

    const cancelled = reduceConnectionState(connecting, { type: 'auth_cancelled' });
    expect(cancelled).toEqual({
      status: 'needs_reconnect',
      availableActions: ['reconnect'],
      operation: 'reconnect',
    });
  });

  it('moves a completed authorization to connected', () => {
    const connecting = reduceConnectionState(disconnected(), { type: 'connect_requested' });
    expect(reduceConnectionState(connecting, { type: 'auth_started' })).toBe(connecting);
    expect(reduceConnectionState(connecting, { type: 'auth_returned' })).toEqual({
      status: 'connected',
      availableActions: ['disconnect'],
    });
  });

  it('turns an authorization timeout into a retryable safe error', () => {
    const connecting = reduceConnectionState(disconnected(), { type: 'connect_requested' });
    const timedOut = reduceConnectionState(connecting, {
      type: 'auth_timed_out',
      diagnosticCode: 'oauth_start_timed_out',
    });
    expect(timedOut).toEqual({
      status: 'error',
      availableActions: ['retry'],
      diagnostic: { code: 'oauth_start_timed_out', source: 'client' },
      operation: 'connect',
    });
    expect(reduceConnectionState(timedOut, { type: 'retry' })).toEqual({
      status: 'connecting',
      availableActions: [],
      operation: 'connect',
    });
  });

  it('tracks disconnect and pending revocation retries', () => {
    const connected = connectionStateFromServer({ state: 'active' });
    const disconnecting = reduceConnectionState(connected, { type: 'disconnect_requested' });
    expect(disconnecting).toMatchObject({ status: 'disconnecting', operation: 'disconnect' });

    const pending = reduceConnectionState(disconnecting, { type: 'revocation_pending' });
    expect(pending).toMatchObject({
      status: 'revocation_pending',
      availableActions: ['retry'],
      operation: 'disconnect',
    });
    expect(reduceConnectionState(pending, { type: 'retry' })).toMatchObject({
      status: 'disconnecting',
      operation: 'disconnect',
    });
  });

  it('ignores actions that are not available', () => {
    const state = disconnected();
    expect(reduceConnectionState(state, { type: 'disconnect_requested' })).toBe(state);
    expect(reduceConnectionState(state, { type: 'retry' })).toBe(state);
  });
});

function disconnected(): ConnectionState {
  return connectionStateFromServer({ state: 'disconnected' });
}
