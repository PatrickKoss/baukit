import { describe, expect, it } from 'vitest';

import { unverifiedSignInSessionFromJwt } from './index.js';

function unsignedJwt(payload: unknown): string {
  const encoded = btoa(JSON.stringify(payload))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/u, '');
  return `header.${encoded}.signature`;
}

describe('unverified sign-in session', () => {
  it('reads a Clerk-shaped token with an active organization', () => {
    expect(
      unverifiedSignInSessionFromJwt(
        unsignedJwt({
          sub: 'user_1',
          sid: 'sess_1',
          v: 2,
          o: { id: 'org_local', rol: 'admin', slg: 'local', per: 'read,manage' },
        }),
      ),
    ).toEqual({
      sessionId: 'sess_1',
      user: { id: 'user_1' },
      organization: {
        id: 'org_local',
        role: 'admin',
        slug: 'local',
        permissions: ['read', 'manage'],
      },
    });
  });

  it('reads a WorkOS-shaped token', () => {
    expect(
      unverifiedSignInSessionFromJwt(
        unsignedJwt({
          sub: 'user_2',
          sid: 'session_2',
          org_id: 'org_local',
          role: 'admin',
          permissions: ['items:read', 7, 'items:write'],
        }),
      ),
    ).toEqual({
      sessionId: 'session_2',
      user: { id: 'user_2' },
      organization: { id: 'org_local', role: 'admin', permissions: ['items:read', 'items:write'] },
    });
  });

  it('omits missing session and organization claims', () => {
    expect(
      unverifiedSignInSessionFromJwt(unsignedJwt({ sub: 'user_3', o: { rol: 'admin' } })),
    ).toEqual({
      user: { id: 'user_3' },
    });
    expect(unverifiedSignInSessionFromJwt(unsignedJwt({ sub: 'user_4', o: 'org' }))).toEqual({
      user: { id: 'user_4' },
    });
    expect(
      unverifiedSignInSessionFromJwt(
        unsignedJwt({ sub: 'user_5', org_id: 'org_5', permissions: 'x' }),
      ),
    ).toEqual({ user: { id: 'user_5' }, organization: { id: 'org_5', permissions: [] } });
    expect(
      unverifiedSignInSessionFromJwt(unsignedJwt({ sub: 'user_6', o: { id: 'org_6' } })),
    ).toEqual({
      user: { id: 'user_6' },
      organization: { id: 'org_6', permissions: [] },
    });
  });

  it('rejects tokens without a subject or a readable payload', () => {
    expect(unverifiedSignInSessionFromJwt(unsignedJwt({ sid: 'sess' }))).toBeUndefined();
    expect(unverifiedSignInSessionFromJwt('not-a-jwt')).toBeUndefined();
  });
});
