import { beforeEach, describe, expect, it, vi } from 'vitest';
import { OidcError } from './index.js';
import { ClerkWebClient } from './clerk.js';
import { WorkOsWebClient } from './workos.js';

const mocks = vi.hoisted(() => ({
  getToken: vi.fn<() => Promise<string | null>>(),
  clerkLoad: vi.fn<() => Promise<void>>(),
  clerkSignIn: vi.fn<() => Promise<void>>(),
  clerkSignOut: vi.fn<() => Promise<void>>(),
  getUser: vi.fn<() => { id: string } | undefined>(),
  getAccessToken: vi.fn<() => Promise<string>>(),
  workosSignIn: vi.fn<() => Promise<void>>(),
  workosSignOut: vi.fn<() => Promise<void>>(),
  refreshFailure: undefined as (() => void) | undefined,
}));
vi.mock('@clerk/clerk-js', () => ({
  Clerk: class {
    session = { getToken: mocks.getToken };
    load = mocks.clerkLoad;
    redirectToSignIn = mocks.clerkSignIn;
    signOut = mocks.clerkSignOut;
  },
}));
vi.mock('@workos-inc/authkit-js', () => ({
  createClient: vi.fn((_id: string, options: { onRefreshFailure: () => void }) => {
    mocks.refreshFailure = options.onRefreshFailure;
    return Promise.resolve({
      getUser: mocks.getUser,
      getAccessToken: mocks.getAccessToken,
      signIn: mocks.workosSignIn,
      signOut: mocks.workosSignOut,
    });
  }),
  LoginRequiredError: class extends Error {},
  RefreshError: class extends Error {
    constructor(public readonly isTransient: boolean) {
      super('refresh');
    }
  },
}));
import { LoginRequiredError } from '@workos-inc/authkit-js';

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getToken.mockResolvedValue('clerk-token');
  mocks.clerkLoad.mockResolvedValue();
  mocks.clerkSignIn.mockResolvedValue();
  mocks.clerkSignOut.mockResolvedValue();
  mocks.getUser.mockReturnValue({ id: 'user_123' });
  mocks.getAccessToken.mockResolvedValue('workos-token');
  mocks.workosSignIn.mockResolvedValue();
  mocks.workosSignOut.mockResolvedValue();
});

describe('Clerk browser contract', () => {
  it('loads once, forwards refresh and template, and uses hosted redirects', async () => {
    const client = new ClerkWebClient({
      publishableKey: 'pk_test',
      redirectUri: 'https://app.test',
      jwtTemplate: 'api',
    });
    expect(await client.handleCallback()).toBe(true);
    expect(await client.accessToken({ forceRefresh: true })).toBe('clerk-token');
    expect(mocks.getToken).toHaveBeenCalledWith({ skipCache: true, template: 'api' });
    await client.login();
    expect(mocks.clerkSignIn).toHaveBeenCalledWith({ signInForceRedirectUrl: 'https://app.test' });
    expect(await client.logout()).toBe(true);
    expect(mocks.clerkSignOut).toHaveBeenCalledWith({ redirectUrl: 'https://app.test' });
    expect(client.hasSession()).toBe(false);
    expect(mocks.clerkLoad).toHaveBeenCalledTimes(1);
  });
  it('emits terminal expiry once and blocks SDK tokens after local clearing', async () => {
    const client = new ClerkWebClient({
      publishableKey: 'pk_test',
      redirectUri: 'https://app.test',
    });
    const expired = vi.fn();
    const unsubscribe = client.subscribeSessionExpired(expired);
    await client.handleCallback();
    mocks.getToken.mockResolvedValue(null);
    expect(await client.accessToken()).toBeUndefined();
    expect(expired).toHaveBeenCalledWith({ type: 'session-expired', reason: 'refresh_rejected' });
    mocks.getToken.mockResolvedValue('stale');
    expect(await client.accessToken()).toBeUndefined();
    expect(mocks.getToken).toHaveBeenCalledTimes(1);
    unsubscribe();
    await client.login();
    mocks.getToken.mockResolvedValue(null);
    await client.accessToken();
    expect(expired).toHaveBeenCalledTimes(1);
  });
});

describe('WorkOS browser contract', () => {
  it('uses AuthKit sign-in, callback, refresh and logout behind the contract', async () => {
    const client = new WorkOsWebClient({ clientId: 'client_app', redirectUri: 'https://app.test' });
    expect(await client.handleCallback()).toBe(true);
    await client.login();
    expect(mocks.workosSignIn).toHaveBeenCalledTimes(1);
    expect(await client.accessToken({ forceRefresh: true })).toBe('workos-token');
    expect(mocks.getAccessToken).toHaveBeenCalledWith({ forceRefresh: true });
    expect(await client.logout()).toBe(true);
    expect(mocks.workosSignOut).toHaveBeenCalledWith({
      returnTo: 'https://app.test',
      navigate: false,
    });
    expect(await client.accessToken()).toBeUndefined();
  });
  it('keeps the session on network failure and clears it on terminal rejection', async () => {
    const client = new WorkOsWebClient({ clientId: 'client_app', redirectUri: 'https://app.test' });
    const expired = vi.fn();
    client.subscribeSessionExpired(expired);
    await client.handleCallback();
    mocks.getAccessToken.mockRejectedValueOnce(new TypeError('network'));
    await expect(client.accessToken()).rejects.toMatchObject<Partial<OidcError>>({
      code: 'refresh_failed',
      retryable: true,
    });
    expect(client.hasSession()).toBe(true);
    expect(expired).not.toHaveBeenCalled();
    mocks.getAccessToken.mockRejectedValueOnce(new LoginRequiredError());
    expect(await client.accessToken()).toBeUndefined();
    expect(client.hasSession()).toBe(false);
    expect(expired).toHaveBeenCalledTimes(1);
  });
  it('observes background SDK refresh failure and supports unsubscribing', async () => {
    const client = new WorkOsWebClient({ clientId: 'client_app', redirectUri: 'https://app.test' });
    const expired = vi.fn();
    const unsubscribe = client.subscribeSessionExpired(expired);
    await client.handleCallback();
    mocks.refreshFailure?.();
    expect(client.hasSession()).toBe(false);
    expect(expired).toHaveBeenCalledTimes(1);
    unsubscribe();
    mocks.refreshFailure?.();
    expect(expired).toHaveBeenCalledTimes(1);
  });
});
