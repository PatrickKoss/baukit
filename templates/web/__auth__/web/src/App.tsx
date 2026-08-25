import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useAriaHiddenInert } from '@baukit/a11y-core/web';
import type { ConsentState } from '@baukit/analytics-core';
import { safeAuthErrorMessage } from '@baukit/auth-web';
import {
  isPersistenceIdentityMismatchError,
  recheckServerSubjectBeforeSyncAdoption,
} from '@baukit/data-contracts';

import { AccessibleDialogExample } from './accessible-dialog';
import { analytics } from './analytics';
import { currentUser, listItems, type CurrentUser, type Item } from './api';
import { authClient } from './auth';
import { backOrReplace, browserNavigation } from './back-or-replace';
import { deriveDetailRouteState } from './route-state';
import { DetailRouteStateView } from './route-state-view';
import { useAuthenticatedLocalData } from './local-data';

const ITEM_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

export function App() {
  useAriaHiddenInert();
  const queryClient = useQueryClient();
  const [authenticated, setAuthenticated] = useState(authClient.hasSession());
  const [sessionExpired, setSessionExpired] = useState(false);
  const [authError, setAuthError] = useState<string>();
  const [user, setUser] = useState<CurrentUser>();
  const localData = useAuthenticatedLocalData(user?.subject, sessionExpired, queryClient);
  const blockIdentityMismatch = localData.blockIdentityMismatch;
  const partition =
    localData.state.status === 'ready' &&
    localData.state.partition.subject === user?.subject &&
    !sessionExpired
      ? localData.state.partition
      : undefined;
  const items = useQuery({
    queryKey: ['items', partition?.subject],
    queryFn: () => listItems(),
    enabled: partition !== undefined,
  });
  const [consent, setConsent] = useState<ConsentState>(analytics.consent);
  const detailId = new URLSearchParams(window.location.search).get('item');
  const detailState = deriveDetailRouteState({
    id: detailId,
    isValidId: (id) => ITEM_ID_PATTERN.test(id),
    loading: items.isPending,
    error: items.error,
    value: items.data?.find((item) => item.id === detailId),
  });

  useEffect(() => {
    const unsubscribe = authClient.subscribeSessionExpired(() => {
      setSessionExpired(true);
      setAuthenticated(false);
      setUser(undefined);
    });
    void authClient
      .handleCallback()
      .then((handled) => {
        if (handled) {
          setSessionExpired(false);
          setAuthenticated(true);
        }
      })
      .catch((cause: unknown) => {
        setAuthError(safeAuthErrorMessage(cause));
      });
    return unsubscribe;
  }, []);

  useEffect(() => {
    if (!authenticated) {
      return;
    }
    let active = true;
    void currentUser()
      .then((nextUser) => {
        if (active) setUser(nextUser);
      })
      .catch((cause: unknown) => {
        if (active) {
          setAuthError(cause instanceof Error ? cause.message : 'Could not confirm account identity.');
        }
      });
    return () => {
      active = false;
    };
  }, [authenticated]);

  useEffect(() => {
    if (partition === undefined || user === undefined) return;
    void recheckServerSubjectBeforeSyncAdoption({
      partitionSubject: partition.subject,
      readServerSubject: () => Promise.resolve(user.subject),
      adopt: () => {
        analytics.identify(user.id);
      },
    }).catch((cause: unknown) => {
      if (isPersistenceIdentityMismatchError(cause)) {
        void blockIdentityMismatch(cause).catch(() => {
          setAuthError('Could not safely close local account data.');
        });
      }
    });
  }, [blockIdentityMismatch, partition, user]);

  useEffect(() => {
    if (items.data !== undefined && consent === 'granted') {
      analytics.capture({ name: 'items_viewed', properties: { count: items.data.length } });
    }
  }, [consent, items.data]);

  function chooseConsent(nextConsent: ConsentState): void {
    analytics.setConsent(nextConsent);
    setConsent(nextConsent);
  }

  async function signOut(): Promise<void> {
    setAuthError(undefined);
    try {
      await localData.clear();
      setAuthenticated(false);
      setUser(undefined);
      await authClient.logout();
    } catch (cause) {
      setAuthError(cause instanceof Error ? cause.message : 'Could not close local account data.');
    }
  }

  return (
    <main className="shell">
      <p className="eyebrow">BAUKIT WEB</p>
      <h1>{{ context.app_name }}</h1>
      <p className="lede">A Vite app using Baukit OIDC discovery and authorization code + PKCE.</p>

      {detailId === null ? null : (
        <DetailRouteStateView<Item>
          state={detailState}
          onExit={() => {
            backOrReplace(browserNavigation(), '/');
          }}
          renderReady={(item) => (
            <section className="panel" aria-labelledby="detail-title">
              <h2 id="detail-title">Item detail</h2>
              <p>{item.name}</p>
              <button
                className="action secondary"
                type="button"
                onClick={() => {
                  backOrReplace(browserNavigation(), '/');
                }}
              >
                Back to items
              </button>
            </section>
          )}
        />
      )}

      <section className="panel" aria-labelledby="identity-title">
        <h2 id="identity-title">Identity</h2>
        {authError === undefined ? null : <p className="error">{authError}</p>}
        {localData.state.status === 'blocked' ? (
          <p className="error" role="alert">
            Local account data is blocked. Sign in again or contact support before continuing.
          </p>
        ) : null}
        {authenticated ? (
          <>
            <p className="muted">
              Signed in as {user?.subject ?? 'confirming…'}; internal user ID{' '}
              {user?.id ?? 'confirming…'}. Local data: {localData.state.status}.
            </p>
            <button
              className="action secondary"
              type="button"
              onClick={() => {
                void signOut();
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
        {localData.state.status === 'initializing' ? (
          <p className="muted" role="status">Preparing local account data…</p>
        ) : null}
        {partition !== undefined && items.isPending ? <p className="muted">Loading items…</p> : null}
        {items.error === null ? null : <p className="error">{items.error.message}</p>}
        {!authenticated ? <p className="muted">Sign in to open your local data partition.</p> : null}
        {partition !== undefined && items.data?.length === 0 ? <p className="muted">No items yet.</p> : null}
        <ul className="items">
          {items.data?.map((item) => (
            <li className="item" key={item.id}>
              <strong>{item.name}</strong>
              <span className="item-id">{item.id}</span>
            </li>
          ))}
        </ul>
      </section>

      <AccessibleDialogExample />

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
