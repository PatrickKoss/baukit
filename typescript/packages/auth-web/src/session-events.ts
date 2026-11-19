export type SessionExpiredReason = 'refresh_rejected' | 'signed_out' | 'profile_erased';

export interface ClearSessionOptions {
  readonly reason?: 'profile_erased';
}

export interface LogoutOptions {
  /**
   * `false` ends only the backend session and leaves the IdP session open, so the caller keeps
   * control and can start a fresh login, as an account switch does. Defaults to `true`.
   */
  readonly endIdpSession?: boolean;
}

export interface SessionExpiredEvent {
  readonly type: 'session-expired';
  readonly reason: SessionExpiredReason;
}

export type SessionExpiredListener = (event: SessionExpiredEvent) => void;
