import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { ConsentState } from '@baukit/analytics-core';

import { analytics } from './analytics';
import { currentUser, listItems } from './api';
import { authClient } from './auth';

export function App() {
  const items = useQuery({ queryKey: ['items'], queryFn: () => listItems() });
  const [authenticated, setAuthenticated] = useState(authClient.hasSession());
  const [authError, setAuthError] = useState<string>();
  const user = useQuery({
    queryKey: ['current-user'],
    queryFn: () => currentUser(),
    enabled: authenticated,
  });
  const [consent, setConsent] = useState<ConsentState>(analytics.consent);

  useEffect(() => {
    void authClient
      .handleCallback()
      .then((handled) => {
        if (handled) {
          setAuthenticated(true);
        }
      })
      .catch((cause: unknown) => {
        setAuthError(cause instanceof Error ? cause.message : 'OIDC login failed.');
      });
  }, []);

  useEffect(() => {
    if (items.data !== undefined && consent === 'granted') {
      analytics.capture({ name: 'items_viewed', properties: { count: items.data.length } });
    }
  }, [consent, items.data]);

  function chooseConsent(nextConsent: ConsentState): void {
    analytics.setConsent(nextConsent);
    setConsent(nextConsent);
  }

  return (
    <main className="shell">
      <p className="eyebrow">BAUKIT WEB</p>
      <h1>{{ context.app_name }}</h1>
      <p className="lede">A Vite app with product-local OIDC authorization code + PKCE.</p>

      <section className="panel" aria-labelledby="identity-title">
        <h2 id="identity-title">Identity</h2>
        {authError === undefined ? null : <p className="error">{authError}</p>}
        {authenticated ? (
          <>
            <p className="muted">
              Signed in as {user.data?.subject ?? 'loading…'}; internal user ID{' '}
              {user.data?.id ?? 'loading…'}.
            </p>
            {user.error === null ? null : <p className="error">{user.error.message}</p>}
            <button
              className="action secondary"
              type="button"
              onClick={() => {
                authClient.logout();
              }}
            >
              Sign out
            </button>
          </>
        ) : (
          <button className="action" type="button" onClick={() => void authClient.login()}>
            Sign in with local Keycloak
          </button>
        )}
      </section>

      <section className="panel" aria-labelledby="items-title">
        <h2 id="items-title">Items</h2>
        {items.isPending ? <p className="muted">Loading items…</p> : null}
        {items.error === null ? null : <p className="error">{items.error.message}</p>}
        {items.data?.length === 0 ? <p className="muted">No items yet.</p> : null}
        <ul className="items">
          {items.data?.map((item) => (
            <li className="item" key={item.id}>
              <strong>{item.name}</strong>
              <span className="item-id">{item.id}</span>
            </li>
          ))}
        </ul>
      </section>

      <aside className="consent" aria-labelledby="privacy-title">
        <h2 id="privacy-title">Analytics privacy</h2>
        <p className="muted">Consent is {consent}. No events leave the app by default.</p>
        <div className="actions">
          <button
            className="action"
            type="button"
            onClick={() => {
              chooseConsent('granted');
            }}
          >
            Allow analytics
          </button>
          <button
            className="action secondary"
            type="button"
            onClick={() => {
              chooseConsent('denied');
            }}
          >
            Deny analytics
          </button>
        </div>
      </aside>
    </main>
  );
}
