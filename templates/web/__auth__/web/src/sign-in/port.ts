import type { SignInSession } from '@baukit/api-runtime';
import type { ClearSessionOptions, SessionExpiredEvent } from '@baukit/auth-web';

export type { SignInOrganization, SignInSession, SignInUser } from '@baukit/api-runtime';

/** The only sign-in surface the app uses. The build picks the adapter behind it. */
export interface SignInPort {
  readonly providerLabel: string;
  readonly hasSession: () => boolean;
  readonly handleCallback: () => Promise<boolean>;
  readonly signIn: () => Promise<void>;
  readonly signOut: () => Promise<boolean>;
  readonly accessToken: (options?: {
    readonly forceRefresh?: boolean;
  }) => Promise<string | undefined>;
  readonly session: () => Promise<SignInSession | undefined>;
  readonly subscribeSessionExpired: (listener: (event: SessionExpiredEvent) => void) => () => void;
  readonly clearSession: (options?: ClearSessionOptions) => void;
}
