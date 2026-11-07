import type { ReactElement } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useAriaHiddenInert } from '@baukit/a11y-core/web';

import { NavigationShell } from './navigation-shell';
import { AccessibleDialogExample } from './accessible-dialog';
import { listItems } from './api';
import { useConsent, useItemsViewedEvent, useLocationSearch } from './app-hooks';
import { ConsentPanel, ItemDetail, ItemList } from './app-panels';

import { PRODUCT_NAME } from './product';

export function App(): ReactElement {
  useAriaHiddenInert();
  const items = useQuery({ queryKey: ['items'], queryFn: () => listItems() });
  const { consent, choose } = useConsent();
  const [search, refreshSearch] = useLocationSearch();
  useItemsViewedEvent(items.data, consent);

  return (
    <>
      <NavigationShell onLocationChange={refreshSearch} />
      <main className="shell">
        <p className="eyebrow">BAUKIT WEB</p>
        <h1>{PRODUCT_NAME}</h1>
        <p className="lede">A small Vite app reading the shared backend through TanStack Query.</p>

        <ItemDetail detailId={new URLSearchParams(search).get('item')} items={items} />

        <section className="panel" aria-labelledby="items-title">
          <h2 id="items-title">Items</h2>
          {items.isPending ? <p className="muted">Loading items…</p> : null}
          {items.error === null ? null : <p className="error">{items.error.message}</p>}
          {items.data?.length === 0 ? <p className="muted">No items yet.</p> : null}
          <ItemList items={items.data} />
        </section>

        <AccessibleDialogExample />

        <ConsentPanel onChoose={choose}>
          Consent is <strong>{consent}</strong>. Events are dropped unless you explicitly allow
          analytics, and the generated app uses a no-op transport by default.
        </ConsentPanel>
      </main>
    </>
  );
}
