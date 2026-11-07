import type { ReactElement } from 'react';

import { authClient } from './auth';
import type { AuthSession } from './auth-session';
import { deleteProfileCopy } from './delete-profile-copy';

function SignedInIdentity({ session }: { readonly session: AuthSession }): ReactElement {
  const { user, localData, signInSession } = session;
  return (
    <>
      <p className="muted">
        Signed in as {user?.subject ?? 'confirming…'}; internal user ID{' '}
        {user?.id ?? 'confirming…'}. Local data: {localData.state.status}.
      </p>
      {signInSession?.organization === undefined ? null : (
        <p className="muted">Organization {signInSession.organization.id}</p>
      )}
      <button
        className="action secondary"
        type="button"
        onClick={() => {
          void session.signOut();
        }}
      >
        Sign out
      </button>
      <a className="action secondary" href="/?page=delete-profile">
        {deleteProfileCopy(navigator.language).title}
      </a>
    </>
  );
}

export function IdentityPanel({ session }: { readonly session: AuthSession }): ReactElement {
  return (
    <section className="panel" aria-labelledby="identity-title">
      <h2 id="identity-title">Identity</h2>
      {session.authError === undefined ? null : <p className="error">{session.authError}</p>}
      {session.localData.state.status === 'blocked' ? (
        <p className="error" role="alert">
          Local account data is blocked. Sign in again or contact support before continuing.
        </p>
      ) : null}
      {session.authenticated ? (
        <SignedInIdentity session={session} />
      ) : (
        <button className="action" type="button" onClick={() => void authClient.signIn()}>
          Sign in with {authClient.providerLabel}
        </button>
      )}
    </section>
  );
}
