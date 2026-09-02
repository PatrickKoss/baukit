export type ConnectionStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'needs_reconnect'
  | 'disconnecting'
  | 'revocation_pending'
  | 'error';

export type ConnectionAction = 'connect' | 'reconnect' | 'disconnect' | 'retry';

export type ConnectionOperation = 'connect' | 'reconnect' | 'disconnect';

export type ServerConnectionState =
  | 'disconnected'
  | 'active'
  | 'connected'
  | 'healthy'
  | 'degraded'
  | 'needs_reconnect'
  | 'failed'
  | 'revoked'
  | 'pending_revocation';

export interface ConnectionDiagnostic {
  readonly code: string;
  readonly source: 'client' | 'server';
}

export interface ConnectionState {
  readonly status: ConnectionStatus;
  readonly availableActions: readonly ConnectionAction[];
  readonly diagnostic?: ConnectionDiagnostic;
  readonly operation?: ConnectionOperation;
}

export interface ServerConnectionSnapshot {
  readonly state: ServerConnectionState;
  readonly diagnosticCode?: string | null;
  readonly providerDiagnostic?: unknown;
}

export type ConnectionEvent =
  | { readonly type: 'server_state'; readonly snapshot: ServerConnectionSnapshot }
  | { readonly type: 'connect_requested' }
  | { readonly type: 'auth_started' }
  | { readonly type: 'auth_returned' }
  | { readonly type: 'auth_cancelled' }
  | { readonly type: 'auth_timed_out'; readonly diagnosticCode?: string }
  | { readonly type: 'disconnect_requested' }
  | { readonly type: 'revocation_pending'; readonly diagnosticCode?: string }
  | { readonly type: 'retry' };

const MAX_DIAGNOSTIC_CODE_LENGTH = 128;
const DIAGNOSTIC_CODE_PATTERN = /^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/;

const ACTIONS = {
  disconnected: ['connect'],
  connecting: [],
  connected: ['disconnect'],
  needs_reconnect: ['reconnect'],
  disconnecting: [],
  revocation_pending: ['retry'],
  error: ['retry'],
} as const satisfies Record<ConnectionStatus, readonly ConnectionAction[]>;

export function connectionStateFromServer(snapshot: ServerConnectionSnapshot): ConnectionState {
  const diagnostic = serverDiagnostic(snapshot.diagnosticCode);
  switch (snapshot.state) {
    case 'active':
    case 'connected':
    case 'healthy':
      return makeConnectionState('connected');
    case 'needs_reconnect':
    case 'revoked':
      return makeConnectionState('needs_reconnect', { diagnostic, operation: 'reconnect' });
    case 'degraded':
    case 'failed':
      return makeConnectionState('error', { diagnostic, operation: 'reconnect' });
    case 'pending_revocation':
      return makeConnectionState('revocation_pending', { diagnostic, operation: 'disconnect' });
    case 'disconnected':
      return makeConnectionState('disconnected');
  }
}

export function reduceConnectionState(
  state: ConnectionState,
  event: ConnectionEvent,
): ConnectionState {
  switch (event.type) {
    case 'server_state':
      return connectionStateFromServer(event.snapshot);
    case 'connect_requested': {
      const action = state.status === 'needs_reconnect' ? 'reconnect' : 'connect';
      if (!state.availableActions.includes(action)) return state;
      return makeConnectionState('connecting', { operation: action });
    }
    case 'auth_started':
      return state.status === 'connecting'
        ? state
        : makeConnectionState('connecting', {
            operation: state.status === 'needs_reconnect' ? 'reconnect' : 'connect',
          });
    case 'auth_returned':
      return makeConnectionState('connected');
    case 'auth_cancelled':
      return state.operation === 'reconnect'
        ? makeConnectionState('needs_reconnect', { operation: 'reconnect' })
        : makeConnectionState('disconnected');
    case 'auth_timed_out':
      return makeConnectionState('error', {
        diagnostic: clientDiagnostic(event.diagnosticCode ?? 'oauth_timed_out'),
        operation: state.operation === 'reconnect' ? 'reconnect' : 'connect',
      });
    case 'disconnect_requested':
      return state.availableActions.includes('disconnect')
        ? makeConnectionState('disconnecting', { operation: 'disconnect' })
        : state;
    case 'revocation_pending':
      return makeConnectionState('revocation_pending', {
        diagnostic: clientDiagnostic(event.diagnosticCode ?? 'revocation_pending'),
        operation: 'disconnect',
      });
    case 'retry':
      if (!state.availableActions.includes('retry')) return state;
      return state.operation === 'disconnect'
        ? makeConnectionState('disconnecting', { operation: 'disconnect' })
        : makeConnectionState('connecting', {
            operation: state.operation === 'reconnect' ? 'reconnect' : 'connect',
          });
  }
}

function makeConnectionState(
  status: ConnectionStatus,
  options: {
    readonly diagnostic?: ConnectionDiagnostic | undefined;
    readonly operation?: ConnectionOperation;
  } = {},
): ConnectionState {
  return {
    status,
    availableActions: ACTIONS[status],
    ...(options.diagnostic === undefined ? {} : { diagnostic: options.diagnostic }),
    ...(options.operation === undefined ? {} : { operation: options.operation }),
  };
}

function serverDiagnostic(code: string | null | undefined): ConnectionDiagnostic | undefined {
  return diagnostic(code, 'server');
}

function clientDiagnostic(code: string | undefined): ConnectionDiagnostic | undefined {
  return diagnostic(code, 'client');
}

function diagnostic(
  code: string | null | undefined,
  source: ConnectionDiagnostic['source'],
): ConnectionDiagnostic | undefined {
  if (
    code === null ||
    code === undefined ||
    code.length > MAX_DIAGNOSTIC_CODE_LENGTH ||
    !DIAGNOSTIC_CODE_PATTERN.test(code)
  ) {
    return undefined;
  }
  return { code, source };
}
