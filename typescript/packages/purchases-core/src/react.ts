import { createContext, createElement, Fragment, useContext, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';

import { featureAccess, type FeatureAccess } from './access.js';
import type { VerifiedEntitlement } from './entitlement.js';
import type { PurchaseFlow, PurchaseState } from './flow.js';
import type { Paywall } from './paywall.js';

interface PurchasesContext {
  readonly flow: PurchaseFlow;
  readonly productId: string;
}

const PurchaseFlowContext = createContext<PurchasesContext | null>(null);

export interface PurchasesProviderProps {
  readonly flow: PurchaseFlow;
  /** The calling product, sent as `from` on Plans links. */
  readonly productId: string;
  readonly children?: ReactNode;
}

export function PurchasesProvider({
  flow,
  productId,
  children,
}: PurchasesProviderProps): ReactNode {
  return createElement(PurchaseFlowContext.Provider, { value: { flow, productId } }, children);
}

function usePurchasesContext(): PurchasesContext {
  const context = useContext(PurchaseFlowContext);
  if (context === null) throw new Error('usePurchaseFlow requires a PurchasesProvider');
  return context;
}

export function usePurchaseFlow(): PurchaseFlow {
  return usePurchasesContext().flow;
}

export function usePurchaseState(): PurchaseState {
  const flow = usePurchaseFlow();
  return useSyncExternalStore(flow.subscribe, flow.getState, flow.getState);
}

export function useEntitlement(): VerifiedEntitlement | null {
  return usePurchaseState().entitlement;
}

/** Access to a feature key from the backend-verified entitlement. See `featureAccess`. */
export function useFeature(key: string): FeatureAccess {
  const { productId } = usePurchasesContext();
  return featureAccess(usePurchaseState(), key, { productId });
}

export interface LockedFeatureProps {
  readonly feature: string;
  readonly children?: ReactNode;
  /** Rendered when the plan lacks the feature, with the C18 paywall decision. */
  readonly locked: (paywall: Paywall) => ReactNode;
  readonly loading?: ReactNode;
  /** Rendered when the plan cannot be read. Offer a retry, never a purchase. */
  readonly unavailable?: ReactNode;
}

/** Renders `children` only while the verified entitlement grants `feature`. */
export function LockedFeature(props: LockedFeatureProps): ReactNode {
  const access = useFeature(props.feature);
  switch (access.status) {
    case 'granted':
      return createElement(Fragment, null, props.children);
    case 'locked':
      return props.locked(access.paywall);
    case 'loading':
      return props.loading ?? null;
    case 'unavailable':
      return props.unavailable ?? null;
  }
}
