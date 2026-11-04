import type {
  ClearSessionOptions,
  OidcSession,
  SessionExpiredEvent,
  SignInOptions,
  SignInResult,
  SignOutResult,
} from '@baukit/auth-native';
import type { PropsWithChildren, ReactNode } from 'react';

export type { SignInOrganization, SignInSession, SignInUser } from '@baukit/api-runtime';

/** The calls the app makes on a sign-in adapter, whichever provider sits behind it. */
export interface SignInClient {
  readonly initialize: () => Promise<OidcSession | undefined>;
  readonly subscribe: (listener: (session: OidcSession | undefined) => void) => () => void;
  readonly subscribeSessionExpired: (listener: (event: SessionExpiredEvent) => void) => () => void;
  readonly accessToken: (options?: {
    readonly forceRefresh?: boolean;
  }) => Promise<string | undefined>;
  readonly signIn: (options?: SignInOptions) => Promise<SignInResult>;
  readonly signOut: () => Promise<SignOutResult>;
  readonly clearSession: (options?: ClearSessionOptions) => Promise<void>;
}

/** One adapter per build. `Provider` wraps the app when the SDK needs React context. */
export interface SignInAdapter {
  readonly providerLabel: string;
  readonly client: SignInClient;
  readonly Provider?: (props: PropsWithChildren) => ReactNode;
}
