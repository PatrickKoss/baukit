{% if context.auth_clerk %}import { ClerkWebClient } from '@baukit/auth-web/clerk';
{% else %}import { WorkOsWebClient } from '@baukit/auth-web/workos';
{% endif %}
import type { SignInPort } from './port';
import { createSignInPort } from './provider-client';

{% if context.auth_clerk %}/** Clerk's browser SDK with hosted sign-in. */
export function createProductionSignIn(): SignInPort {
  return createSignInPort(
    'Clerk',
    () =>
      new ClerkWebClient({
        publishableKey: import.meta.env.VITE_CLERK_PUBLISHABLE_KEY ?? '',
        redirectUri: `${window.location.origin}/`,
      }),
  );
}
{% else %}/** WorkOS AuthKit's browser SDK as a public client. */
export function createProductionSignIn(): SignInPort {
  return createSignInPort(
    'WorkOS',
    () =>
      new WorkOsWebClient({
        clientId: import.meta.env.VITE_WORKOS_CLIENT_ID ?? '',
        redirectUri: `${window.location.origin}/`,
      }),
  );
}
{% endif %}