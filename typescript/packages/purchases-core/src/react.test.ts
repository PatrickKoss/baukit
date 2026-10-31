// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { createElement, type ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';

import { createNoopPurchasesPort, PurchaseFlow, type VerifiedEntitlement } from './index.js';
import {
  PurchasesProvider,
  useEntitlement,
  useFeature,
  usePurchaseFlow,
  usePurchaseState,
} from './react.js';

const SUBJECT = 'suite|person-1';

afterEach(() => {
  cleanup();
});

function setup() {
  let current: VerifiedEntitlement | null = null;
  const flow = new PurchaseFlow({
    port: createNoopPurchasesPort(),
    backend: { refreshEntitlement: () => Promise.resolve(current) },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(PurchasesProvider, { flow }, children);
  const grant = (features: string[], revision: number) => {
    current = { suiteSubject: SUBJECT, tier: 'premium', features, revision };
  };
  return { flow, wrapper, grant };
}

describe('purchase hooks', () => {
  it('gates on the refreshed entitlement feature keys', async () => {
    const { flow, wrapper, grant } = setup();
    const view = renderHook(
      () => ({ exports: useFeature('export'), entitlement: useEntitlement() }),
      {
        wrapper,
      },
    );
    expect(view.result.current.exports).toBe(false);

    await act(async () => {
      await flow.identify(SUBJECT);
      grant(['export'], 2);
      await flow.refreshEntitlement();
    });
    expect(view.result.current.exports).toBe(true);
    expect(view.result.current.entitlement?.revision).toBe(2);

    await act(async () => {
      await flow.signOut();
    });
    expect(view.result.current.exports).toBe(false);
  });

  it('exposes the flow and its state', async () => {
    const { flow, wrapper } = setup();
    const view = renderHook(() => ({ flow: usePurchaseFlow(), state: usePurchaseState() }), {
      wrapper,
    });

    expect(view.result.current.flow).toBe(flow);
    await act(async () => {
      await flow.identify(SUBJECT);
    });
    expect(view.result.current.state.suiteSubject).toBe(SUBJECT);
  });

  it('requires a provider', () => {
    expect(() => renderHook(() => usePurchaseFlow())).toThrow(
      'usePurchaseFlow requires a PurchasesProvider',
    );
  });
});
