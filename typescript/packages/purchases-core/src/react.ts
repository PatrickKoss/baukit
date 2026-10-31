import { createContext, createElement, useContext, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';

import { hasFeature, type VerifiedEntitlement } from './entitlement.js';
import type { PurchaseFlow, PurchaseState } from './flow.js';

const PurchaseFlowContext = createContext<PurchaseFlow | null>(null);

export interface PurchasesProviderProps {
  readonly flow: PurchaseFlow;
  readonly children?: ReactNode;
}

export function PurchasesProvider({ flow, children }: PurchasesProviderProps): ReactNode {
  return createElement(PurchaseFlowContext.Provider, { value: flow }, children);
}

export function usePurchaseFlow(): PurchaseFlow {
  const flow = useContext(PurchaseFlowContext);
  if (flow === null) throw new Error('usePurchaseFlow requires a PurchasesProvider');
  return flow;
}

export function usePurchaseState(): PurchaseState {
  const flow = usePurchaseFlow();
  return useSyncExternalStore(flow.subscribe, flow.getState, flow.getState);
}

export function useEntitlement(): VerifiedEntitlement | null {
  return usePurchaseState().entitlement;
}

/** True only when the backend-verified entitlement grants the feature key. */
export function useFeature(key: string): boolean {
  return hasFeature(useEntitlement(), key);
}
