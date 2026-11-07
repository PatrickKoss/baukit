export type SessionExpiredReason = 'refresh_rejected' | 'signed_out' | 'profile_erased';

export interface ClearSessionOptions {
  readonly reason?: 'profile_erased';
}

export interface SessionExpiredEvent {
  readonly type: 'session-expired';
  readonly reason: SessionExpiredReason;
}

export type SessionExpiredListener = (event: SessionExpiredEvent) => void;
