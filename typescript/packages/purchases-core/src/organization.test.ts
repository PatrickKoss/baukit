// @vitest-environment jsdom
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  AccountPlanError,
  ACCOUNT_PLAN_ERROR_CODES,
  createAccountPlanBackend,
  createNoopPurchasesPort,
  ORGANIZATION_HEADER,
  organizationFeatureAccess,
  paywallFor,
  PurchaseFlow,
  type VerifiedEntitlement,
} from './index.js';
import { PurchasesProvider, useFeature, useOrganizationPlan } from './react.js';

const STUDIO = '00000000-0000-0000-0000-00000000000a';
const AGENCY = '00000000-0000-0000-0000-00000000000b';
const PRIVACY = 'ahp.privacy';
const AUTOMATION = 'ahp.automation';
const context = { productId: 'ahp' };

function organizationPlan(
  organizationId: string,
  tier: string,
  features: string[],
  revision = 1,
): VerifiedEntitlement {
  return {
    mode: 'hub',
    suiteSubject: 'suite|alice',
    tier,
    features,
    revision,
    plansUrl: null,
    billing: null,
    organization: { organizationId, name: `Org ${organizationId.slice(-1)}` },
  };
}
const studio = organizationPlan(STUDIO, 'business', [PRIVACY, AUTOMATION]);
const agency = organizationPlan(AGENCY, 'starter', [PRIVACY]);

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}
function failure(status: number, code: string): Response {
  return json(status, { error: { code, message: code, details: {} } });
}

afterEach(() => {
  cleanup();
});

describe('paywallFor with an organization subject', () => {
  it('gives one account each organization its own tier', () => {
    expect(paywallFor(studio, { ...context, feature: AUTOMATION })).toEqual({
      kind: 'covered',
      channel: 'organization',
    });
    expect(paywallFor(agency, { ...context, feature: AUTOMATION })).toEqual({
      kind: 'organization_managed',
      organizationId: AGENCY,
      organizationName: 'Org b',
    });
    expect(paywallFor(agency, context)).toEqual({ kind: 'covered', channel: 'organization' });
  });

  it('never sells a plan to a member, whatever the billing claim says', () => {
    const billed = {
      ...agency,
      plansUrl: 'https://accounts.example.test/plans',
      billing: { channel: 'web' as const },
    };
    expect(paywallFor(billed, { ...context, feature: AUTOMATION }).kind).toBe(
      'organization_managed',
    );
    expect(paywallFor({ ...agency, mode: 'standalone' }, context)).toEqual({ kind: 'none' });
  });
});

describe('organizationFeatureAccess', () => {
  it('grants, locks and fails closed per organization state', () => {
    expect(
      organizationFeatureAccess({ status: 'ready', entitlement: studio }, AUTOMATION, context),
    ).toEqual({ status: 'granted', entitlement: studio });
    expect(
      organizationFeatureAccess({ status: 'ready', entitlement: agency }, AUTOMATION, context),
    ).toMatchObject({ status: 'locked', paywall: { kind: 'organization_managed' } });
    expect(organizationFeatureAccess({ status: 'loading' }, PRIVACY, context)).toEqual({
      status: 'loading',
    });
    expect(organizationFeatureAccess({ status: 'unavailable' }, PRIVACY, context)).toEqual({
      status: 'unavailable',
    });
    expect(organizationFeatureAccess({ status: 'not_accessible' }, PRIVACY, context)).toEqual({
      status: 'locked',
      entitlement: null,
      paywall: { kind: 'none' },
    });
  });
});

describe('createAccountPlanBackend organizationEntitlement', () => {
  it('names the organization in the header and parses its plan', async () => {
    const fetch = vi.fn((_url: string, init: { headers: Record<string, string> }) =>
      Promise.resolve(json(200, init.headers[ORGANIZATION_HEADER] === STUDIO ? studio : agency)),
    );
    const backend = createAccountPlanBackend({
      baseUrl: 'https://api.example.test/api/v1',
      fetch,
      headers: () => ({ authorization: 'Bearer t' }),
    });
    await expect(backend.organizationEntitlement?.(STUDIO)).resolves.toEqual(studio);
    await expect(backend.organizationEntitlement?.(AGENCY)).resolves.toEqual(agency);
    expect(fetch).toHaveBeenCalledWith('https://api.example.test/api/v1/me/entitlement', {
      headers: {
        accept: 'application/json',
        authorization: 'Bearer t',
        [ORGANIZATION_HEADER]: STUDIO,
      },
    });
  });

  it('rejects a non-member and a malformed organization body', async () => {
    expect(ACCOUNT_PLAN_ERROR_CODES).toContain('organization_not_accessible');
    const responses = [
      failure(403, 'organization_not_accessible'),
      json(200, { ...studio, organization: { organizationId: 7 } }),
    ];
    const backend = createAccountPlanBackend({
      baseUrl: 'https://api.example.test',
      fetch: () => Promise.resolve(responses.shift() ?? failure(500, 'internal_error')),
    });
    await expect(backend.organizationEntitlement?.(STUDIO)).rejects.toEqual(
      new AccountPlanError(403, 'organization_not_accessible'),
    );
    await expect(backend.organizationEntitlement?.(STUDIO)).rejects.toThrow(TypeError);
  });
});

describe('useFeature with an organization plan', () => {
  function wrapper({ children }: { children: ReactNode }) {
    const flow = new PurchaseFlow({
      port: createNoopPurchasesPort(),
      backend: { refreshEntitlement: () => Promise.resolve(null) },
    });
    return createElement(PurchasesProvider, { flow, productId: 'ahp' }, children);
  }

  it('gates without a suite session, as for an organization token', async () => {
    const backend = { organizationEntitlement: () => Promise.resolve(studio) };
    const view = renderHook(
      () => useFeature(AUTOMATION, { organization: useOrganizationPlan(backend, STUDIO) }),
      { wrapper },
    );
    expect(view.result.current).toEqual({ status: 'loading' });
    await waitFor(() => {
      expect(view.result.current.status).toBe('granted');
    });
  });

  it('locks a lost grant on the next read and hides a non-member', async () => {
    let current: VerifiedEntitlement = studio;
    const backend = {
      organizationEntitlement: (id: string) =>
        id === STUDIO
          ? Promise.resolve(current)
          : Promise.reject(new AccountPlanError(403, 'organization_not_accessible')),
    };
    const view = renderHook(
      ({ id }: { id: string }) =>
        useFeature(AUTOMATION, { organization: useOrganizationPlan(backend, id) }),
      { wrapper, initialProps: { id: STUDIO } },
    );
    await waitFor(() => {
      expect(view.result.current.status).toBe('granted');
    });
    current = organizationPlan(STUDIO, 'starter', [PRIVACY], 2);
    view.rerender({ id: AGENCY });
    await waitFor(() => {
      expect(view.result.current).toEqual({
        status: 'locked',
        entitlement: null,
        paywall: { kind: 'none' },
      });
    });
    view.rerender({ id: STUDIO });
    await waitFor(() => {
      expect(view.result.current).toMatchObject({
        status: 'locked',
        paywall: { kind: 'organization_managed', organizationId: STUDIO },
      });
    });
  });

  it('reports an unreachable hub as unavailable', async () => {
    const backend = {
      organizationEntitlement: () =>
        Promise.reject(new AccountPlanError(503, 'entitlements_unavailable', 5)),
    };
    const view = renderHook(
      () => useFeature(PRIVACY, { organization: useOrganizationPlan(backend, STUDIO) }),
      { wrapper },
    );
    await waitFor(() => {
      expect(view.result.current).toEqual({ status: 'unavailable' });
    });
  });
});
