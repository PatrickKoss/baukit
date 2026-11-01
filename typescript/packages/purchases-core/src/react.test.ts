// @vitest-environment jsdom
import { act, cleanup, render, renderHook } from '@testing-library/react';
import { createElement, type ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';

import { createNoopPurchasesPort, PurchaseFlow, type VerifiedEntitlement } from './index.js';
import {
  LockedFeature,
  PurchasesProvider,
  useEntitlement,
  useFeature,
  usePurchaseFlow,
  usePurchaseState,
} from './react.js';

const SUBJECT = 'suite|person-1';
const PLANS = 'https://accounts.example.test/plans';

afterEach(() => {
  cleanup();
});

function setup() {
  let current: VerifiedEntitlement | null = null;
  let fail = false;
  const flow = new PurchaseFlow({
    port: createNoopPurchasesPort(),
    backend: {
      refreshEntitlement: () =>
        fail ? Promise.reject(new Error('offline')) : Promise.resolve(current),
    },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(PurchasesProvider, { flow, productId: 'hebkit' }, children);
  const grant = (
    features: string[],
    revision: number,
    billing: VerifiedEntitlement['billing'] = null,
  ) => {
    current = {
      mode: 'hub',
      suiteSubject: SUBJECT,
      tier: 'premium',
      features,
      revision,
      plansUrl: PLANS,
      billing,
    };
  };
  const failNext = () => {
    fail = true;
  };
  return { flow, wrapper, grant, failNext };
}

describe('purchase hooks', () => {
  it('gates on the refreshed entitlement feature keys', async () => {
    const { flow, wrapper, grant } = setup();
    const view = renderHook(
      () => ({ exports: useFeature('export'), entitlement: useEntitlement() }),
      { wrapper },
    );
    expect(view.result.current.exports).toEqual({
      status: 'locked',
      entitlement: null,
      paywall: { kind: 'none' },
    });

    await act(async () => {
      await flow.identify(SUBJECT);
    });
    expect(view.result.current.exports).toEqual({ status: 'loading' });

    await act(async () => {
      grant(['sync'], 1);
      await flow.refreshEntitlement();
    });
    expect(view.result.current.exports).toMatchObject({
      status: 'locked',
      paywall: { kind: 'purchase', plansUrl: `${PLANS}?from=hebkit&feature=export` },
    });

    await act(async () => {
      grant(['export'], 2);
      await flow.refreshEntitlement();
    });
    expect(view.result.current.exports.status).toBe('granted');
    expect(view.result.current.entitlement?.revision).toBe(2);

    await act(async () => {
      await flow.signOut();
    });
    expect(view.result.current.exports.status).toBe('locked');
  });

  it('reports an unreadable plan as unavailable, never as granted or for sale', async () => {
    const { flow, wrapper, failNext } = setup();
    const view = renderHook(() => useFeature('export'), { wrapper });
    await act(async () => {
      await flow.identify(SUBJECT);
      failNext();
      await flow.refreshEntitlement().catch(() => undefined);
    });
    expect(view.result.current).toEqual({ status: 'unavailable' });
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

describe('LockedFeature', () => {
  function view(wrapper: (props: { children: ReactNode }) => ReactNode) {
    return render(
      createElement(LockedFeature, {
        feature: 'export',
        locked: (paywall) => `locked:${paywall.kind}`,
        loading: 'loading',
        unavailable: 'retry',
        children: 'export',
      }),
      { wrapper },
    );
  }

  it('renders each state', async () => {
    const { flow, wrapper, grant, failNext } = setup();
    const screen = view(wrapper);
    expect(screen.container.textContent).toBe('locked:none');
    await act(async () => {
      await flow.identify(SUBJECT);
    });
    expect(screen.container.textContent).toBe('loading');
    await act(async () => {
      grant([], 1, { channel: 'play_store', soldBy: 'eigenruhe' });
      await flow.refreshEntitlement();
    });
    expect(screen.container.textContent).toBe('locked:manage');
    await act(async () => {
      grant(['export'], 2);
      await flow.refreshEntitlement();
    });
    expect(screen.container.textContent).toBe('export');
    await act(async () => {
      await flow.signOut();
      await flow.identify(SUBJECT);
      failNext();
      await flow.refreshEntitlement().catch(() => undefined);
    });
    expect(screen.container.textContent).toBe('retry');
  });

  it('renders nothing for loading and unavailable by default', async () => {
    const { flow, wrapper } = setup();
    const screen = render(
      createElement(LockedFeature, { feature: 'export', locked: () => 'locked', children: 'x' }),
      { wrapper },
    );
    await act(async () => {
      await flow.identify(SUBJECT);
    });
    expect(screen.container.textContent).toBe('');
  });
});
