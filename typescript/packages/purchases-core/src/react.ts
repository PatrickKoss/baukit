import {
  createContext,
  createElement,
  Fragment,
  useCallback,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
} from 'react';
import type { ReactNode } from 'react';

import {
  featureAccess,
  organizationFeatureAccess,
  type FeatureAccess,
  type OrganizationPlanState,
} from './access.js';
import { AccountPlanError } from './account-plan.js';
import type { EntitlementBackend, VerifiedEntitlement } from './entitlement.js';
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

export interface FeatureOptions {
  /** Check the organization's plan from `useOrganizationPlan` instead of the account's. */
  readonly organization?: OrganizationPlanState;
}

/** Access to a feature key from the backend-verified entitlement. See `featureAccess`. */
export function useFeature(key: string, options: FeatureOptions = {}): FeatureAccess {
  const { productId } = usePurchasesContext();
  const state = usePurchaseState();
  if (options.organization !== undefined)
    return organizationFeatureAccess(options.organization, key, { productId });
  return featureAccess(state, key, { productId });
}

function organizationFailure(error: unknown): OrganizationPlanState {
  if (error instanceof AccountPlanError && error.code === 'organization_not_accessible')
    return { status: 'not_accessible' };
  return { status: 'unavailable' };
}

interface SettledOrganizationPlan {
  readonly backend: Pick<EntitlementBackend, 'organizationEntitlement'>;
  readonly organizationId: string;
  readonly attempt: number;
  readonly state: OrganizationPlanState;
}

/** An organization's plan state with `retry`, which reads the plan again. */
export type OrganizationPlan = OrganizationPlanState & { readonly retry: () => void };

/** Loads the plan of the organization the account acts for. */
export function useOrganizationPlan(
  backend: Pick<EntitlementBackend, 'organizationEntitlement'>,
  organizationId: string,
): OrganizationPlan {
  const [attempt, setAttempt] = useState(0);
  const [settled, setSettled] = useState<SettledOrganizationPlan | null>(null);
  const retry = useCallback(() => {
    setAttempt((previous) => previous + 1);
  }, []);
  useEffect(() => {
    let active = true;
    const settle = (state: OrganizationPlanState): void => {
      if (active) setSettled({ backend, organizationId, attempt, state });
    };
    const load = backend.organizationEntitlement?.(organizationId) ?? Promise.resolve(null);
    load.then(
      (entitlement) => {
        settle(
          entitlement === null ? { status: 'not_accessible' } : { status: 'ready', entitlement },
        );
      },
      (error: unknown) => {
        settle(organizationFailure(error));
      },
    );
    return () => {
      active = false;
    };
  }, [backend, organizationId, attempt]);
  const current =
    settled?.backend === backend &&
    settled.organizationId === organizationId &&
    settled.attempt === attempt;
  const state: OrganizationPlanState = current ? settled.state : { status: 'loading' };
  return { ...state, retry };
}

export interface LockedFeatureProps {
  readonly feature: string;
  readonly children?: ReactNode;
  /** Rendered when the plan lacks the feature, with the C18 paywall decision. */
  readonly locked: (paywall: Paywall) => ReactNode;
  readonly loading?: ReactNode;
  /** Rendered when the plan cannot be read. Offer a retry, never a purchase. */
  readonly unavailable?: ReactNode;
  readonly organization?: OrganizationPlanState;
}

/** Renders `children` only while the verified entitlement grants `feature`. */
export function LockedFeature(props: LockedFeatureProps): ReactNode {
  const access = useFeature(
    props.feature,
    props.organization === undefined ? {} : { organization: props.organization },
  );
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
