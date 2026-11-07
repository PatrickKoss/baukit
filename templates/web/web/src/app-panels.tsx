import type { ReactElement, ReactNode } from 'react';
import type { ConsentState } from '@baukit/analytics-core';

import type { Item } from './api';
import { backOrReplace, browserNavigation } from './back-or-replace';
import { deriveDetailRouteState } from './route-state';
import { DetailRouteStateView } from './route-state-view';

const ITEM_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function backToItems(): void {
  backOrReplace(browserNavigation(), '/');
}

export interface ItemsResult {
  readonly isPending: boolean;
  readonly error: Error | null;
  readonly data: readonly Item[] | undefined;
}

/** The `?item=` detail route; renders nothing when no item is selected. */
export function ItemDetail({
  detailId,
  items,
}: {
  readonly detailId: string | null;
  readonly items: ItemsResult;
}): ReactElement | null {
  if (detailId === null) {
    return null;
  }
  const state = deriveDetailRouteState({
    id: detailId,
    isValidId: (id) => ITEM_ID_PATTERN.test(id),
    loading: items.isPending,
    error: items.error,
    value: items.data?.find((item) => item.id === detailId),
  });
  return (
    <DetailRouteStateView<Item>
      state={state}
      onExit={backToItems}
      renderReady={(item) => (
        <section className="panel" aria-labelledby="detail-title">
          <h2 id="detail-title">Item detail</h2>
          <p>{item.name}</p>
          <button className="action secondary" type="button" onClick={backToItems}>
            Back to items
          </button>
        </section>
      )}
    />
  );
}

export function ItemList({ items }: { readonly items: readonly Item[] | undefined }): ReactElement {
  return (
    <ul className="items">
      {items?.map((item) => (
        <li className="item" key={item.id}>
          <strong>{item.name}</strong>
          <span className="item-id">{item.id}</span>
        </li>
      ))}
    </ul>
  );
}

export function ConsentPanel({
  onChoose,
  children,
}: {
  readonly onChoose: (next: ConsentState) => void;
  readonly children: ReactNode;
}): ReactElement {
  return (
    <aside className="consent" aria-labelledby="privacy-title">
      <h2 id="privacy-title">Analytics privacy</h2>
      <p className="muted">{children}</p>
      <div className="actions">
        <button
          className="action"
          type="button"
          onClick={() => {
            onChoose('granted');
          }}
        >
          Allow analytics
        </button>
        <button
          className="action secondary"
          type="button"
          onClick={() => {
            onChoose('denied');
          }}
        >
          Deny analytics
        </button>
      </div>
    </aside>
  );
}
