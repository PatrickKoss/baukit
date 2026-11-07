import type { ReactElement } from 'react';
import { useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query';
import { useAriaHiddenInert } from '@baukit/a11y-core/web';

import { AccessibleDialogExample } from './accessible-dialog';
import { NavigationShell } from './navigation-shell';
import { listItems, type Item } from './api';
import { useConsent, useItemsViewedEvent, useLocationSearch } from './app-hooks';
import { ConsentPanel, ItemDetail, ItemList } from './app-panels';
import { useAuthSession, type AuthSession } from './auth-session';
import { IdentityPanel } from './identity-panel';
import { DeleteProfileRoute } from './delete-profile-route';
import { deleteProfileCopy } from './delete-profile-copy';

import { PRODUCT_NAME } from './product';

function ItemsPanel({
  session,
  items,
}: {
  readonly session: AuthSession;
  readonly items: UseQueryResult<readonly Item[]>;
}): ReactElement {
  const { partition } = session;
  return (
    <section className="panel" aria-labelledby="items-title">
      <h2 id="items-title">Items</h2>
      {session.localData.state.status === 'initializing' ? (
        <p className="muted" role="status">Preparing local account data…</p>
      ) : null}
      {partition !== undefined && items.isPending ? <p className="muted">Loading items…</p> : null}
      {items.error === null ? null : <p className="error">{items.error.message}</p>}
      {!session.authenticated ? <p className="muted">Sign in to open your local data partition.</p> : null}
      {partition !== undefined && items.data?.length === 0 ? <p className="muted">No items yet.</p> : null}
      <ItemList items={items.data} />
    </section>
  );
}

function AccountNavigation({
  session,
  onLocationChange,
}: {
  readonly session: AuthSession;
  readonly onLocationChange: () => void;
}): ReactElement {
  return (
{% raw %}
    <NavigationShell
      onLocationChange={onLocationChange}
      profile={{
        label: 'Account',
        initials: 'A',
        menu: [
          { id: 'account', label: 'Profile', href: '/#identity-title' },
          {
            id: 'delete-profile',
            label: deleteProfileCopy(navigator.language).title,
            href: '/?page=delete-profile',
          },
          {
            id: 'signout',
            label: 'Sign out',
            onSelect: () => {
              void session.signOut();
            },
          },
        ],
      }}
    />
{% endraw %}
  );
}

export function App(): ReactElement {
  useAriaHiddenInert();
  const session = useAuthSession(useQueryClient());
  const { partition, user } = session;
  const items = useQuery({
    queryKey: ['items', partition?.subject],
    queryFn: () => listItems(),
    enabled: partition !== undefined,
  });
  const { consent, choose } = useConsent();
  const [search, refreshSearch] = useLocationSearch();
  const searchParams = new URLSearchParams(search);
  useItemsViewedEvent(items.data, consent);

  return (
    <main className="shell">
      <AccountNavigation session={session} onLocationChange={refreshSearch} />
      <p className="eyebrow">BAUKIT WEB</p>
      <h1>{PRODUCT_NAME}</h1>
      <p className="lede">Sign in to open your account.</p>

      {searchParams.get('page') === 'delete-profile' ? (
        <DeleteProfileRoute
          subject={user?.subject}
          ready={partition !== undefined}
          eraseLocalPartition={() => session.localData.erase(user?.id ?? '')}
          onSignedOut={session.markSignedOut}
        />
      ) : null}

      <ItemDetail detailId={searchParams.get('item')} items={items} />
      <IdentityPanel session={session} />
      <ItemsPanel session={session} items={items} />
      <AccessibleDialogExample />

      <ConsentPanel onChoose={choose}>
        Consent is {consent}. No events leave the app by default.
      </ConsentPanel>
    </main>
  );
}
