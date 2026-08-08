import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { ConsentState } from '@baukit/analytics-core';

import { analytics } from './analytics';
import { listItems } from './api';

export function App() {
  const items = useQuery({ queryKey: ['items'], queryFn: () => listItems() });
  const [consent, setConsent] = useState<ConsentState>(analytics.consent);

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
      <p className="lede">A small Vite app reading the shared backend through TanStack Query.</p>

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
        <p className="muted">
          Consent is <strong>{consent}</strong>. Events are dropped unless you explicitly allow
          analytics, and the generated app uses a no-op transport by default.
        </p>
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
