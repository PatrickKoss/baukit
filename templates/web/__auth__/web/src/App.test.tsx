// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { SessionExpiredEvent } from '@baukit/auth-web';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  currentUser: vi.fn(),
  listItems: vi.fn(() => Promise.resolve([])),
  identify: vi.fn(),
  subscribeSessionExpired: vi.fn(),
}));
vi.mock('./api', () => ({ currentUser: mocks.currentUser, listItems: mocks.listItems }));
vi.mock('./auth', () => ({
  authClient: {
    hasSession: () => true,
    handleCallback: () => Promise.resolve(false),
    subscribeSessionExpired: mocks.subscribeSessionExpired,
    login: vi.fn(),
    logout: vi.fn(),
  },
}));
vi.mock('./analytics', () => ({
  analytics: { consent: 'denied', reset: vi.fn(), identify: mocks.identify, capture: vi.fn() },
}));
vi.mock('./navigation-shell', () => ({ NavigationShell: () => null }));
vi.mock('./accessible-dialog', () => ({ AccessibleDialogExample: () => null }));

import { App } from './App';

let expired: ((event: SessionExpiredEvent) => void) | undefined;
beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  mocks.subscribeSessionExpired.mockImplementation((listener: (event: SessionExpiredEvent) => void) => {
    expired = listener;
    return () => { expired = undefined; };
  });
});
afterEach(cleanup);

async function renderProfile(id: string) {
  mocks.currentUser.mockResolvedValue({ id, subject: 'same-oidc-subject' });
  const queryClient = new QueryClient();
  const rendered = render(<QueryClientProvider client={queryClient}><App /></QueryClientProvider>);
  await waitFor(() => { expect(mocks.identify).toHaveBeenCalledWith(id); });
  return { ...rendered, queryClient };
}

it('selects the new server profile for the same auth subject on another login', async () => {
  const first = await renderProfile('old-profile');
  first.unmount();
  await renderProfile('new-profile');

  const registry = Object.keys(localStorage).find((key) => key.endsWith(':local-data-registry:v1'));
  const serialized = localStorage.getItem(registry ?? '');
  expect(serialized).toContain('old-profile');
  expect(serialized).toContain('new-profile');
  expect(serialized).not.toContain('same-oidc-subject');
  expect(mocks.listItems).toHaveBeenCalledTimes(2);
});

it('passes the expiry reason through and erases the active profile', async () => {
  const { queryClient } = await renderProfile('erased-profile');
  queryClient.setQueryData(['private'], 'erased content');

  act(() => { expired?.({ type: 'session-expired', reason: 'profile_erased' }); });

  await waitFor(() => {
    const registry = Object.keys(localStorage).find((key) => key.endsWith(':local-data-registry:v1'));
    expect(localStorage.getItem(registry ?? '')).not.toContain('erased-profile');
  });
  expect(queryClient.getQueryData(['private'])).toBeUndefined();
});
