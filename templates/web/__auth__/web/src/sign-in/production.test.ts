import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const sdk = vi.hoisted(() => ({
  constructed: [] as unknown[],
  token: vi.fn<() => Promise<string | null>>(),
  signIn: vi.fn(() => Promise.resolve()),
  signOut: vi.fn(() => Promise.resolve()),
}));

{% if context.auth_clerk %}vi.mock('@clerk/clerk-js', () => ({
  Clerk: class {
    public session: { getToken: typeof sdk.token } | null = null;
    public constructor(publishableKey: string) {
      sdk.constructed.push(publishableKey);
    }
    public load(): Promise<void> {
      this.session = { getToken: sdk.token };
      return Promise.resolve();
    }
    public redirectToSignIn = sdk.signIn;
    public signOut = sdk.signOut;
  },
}));
{% else %}vi.mock('@workos-inc/authkit-js', () => ({
  LoginRequiredError: class extends Error {},
  RefreshError: class extends Error {},
  createClient: (clientId: string, options: unknown) => {
    sdk.constructed.push({ clientId, options });
    return Promise.resolve({
      getUser: () => ({ id: 'user_1' }),
      getAccessToken: sdk.token,
      signIn: sdk.signIn,
      signOut: sdk.signOut,
    });
  },
}));
{% endif %}
{% if context.auth_workos %}import { LoginRequiredError } from '@workos-inc/authkit-js';
{% endif %}
import { createProductionSignIn } from './production';

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload)).replaceAll('+', '-').replaceAll('/', '_');
  return `header.${encoded.replace(/=+$/u, '')}.signature`;
}

beforeEach(() => {
  sdk.constructed.length = 0;
  vi.clearAllMocks();
  vi.stubGlobal('window', { location: { origin: 'https://app.example.test' } });
  vi.stubEnv('{{ "VITE_CLERK_PUBLISHABLE_KEY" if context.auth_clerk else "VITE_WORKOS_CLIENT_ID" }}', 'public-provider-id');
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

describe('{{ "Clerk" if context.auth_clerk else "WorkOS" }} production sign-in adapter', () => {
  it('maps the SDK session onto the port', async () => {
{% if context.auth_clerk %}    sdk.token.mockResolvedValue(
      unsignedJwt({ sub: 'user_1', sid: 'sess_1', o: { id: 'org_1', rol: 'admin', slg: 'acme', per: 'read,manage' } }),
    );
{% else %}    sdk.token.mockResolvedValue(
      unsignedJwt({ sub: 'user_1', sid: 'session_1', org_id: 'org_1', role: 'admin', permissions: ['read'] }),
    );
{% endif %}    const signIn = createProductionSignIn();

    expect(signIn.providerLabel).toBe('{{ "Clerk" if context.auth_clerk else "WorkOS" }}');
    await expect(signIn.handleCallback()).resolves.toBe(true);
    await expect(signIn.session()).resolves.toEqual({
{% if context.auth_clerk %}      sessionId: 'sess_1',
      user: { id: 'user_1' },
      organization: { id: 'org_1', role: 'admin', slug: 'acme', permissions: ['read', 'manage'] },
{% else %}      sessionId: 'session_1',
      user: { id: 'user_1' },
      organization: { id: 'org_1', role: 'admin', permissions: ['read'] },
{% endif %}    });
{% if context.auth_clerk %}    expect(sdk.constructed).toEqual(['public-provider-id']);
{% else %}    expect(sdk.constructed).toEqual([
      { clientId: 'public-provider-id', options: expect.objectContaining({ redirectUri: 'https://app.example.test/' }) as unknown },
    ]);
{% endif %}  });

  it('starts hosted sign-in and signs out through the SDK', async () => {
    const signIn = createProductionSignIn();

    await signIn.signIn();
    await expect(signIn.signOut()).resolves.toBe(true);

    expect(sdk.signIn).toHaveBeenCalledOnce();
    expect(sdk.signOut).toHaveBeenCalledOnce();
    expect(signIn.hasSession()).toBe(false);
  });

  it('reports no session when the SDK has no token', async () => {
{% if context.auth_clerk %}    sdk.token.mockResolvedValue(null);
{% else %}    sdk.token.mockRejectedValue(new LoginRequiredError());
{% endif %}    const signIn = createProductionSignIn();
    await signIn.handleCallback();

    await expect(signIn.session()).resolves.toBeUndefined();
  });
});
