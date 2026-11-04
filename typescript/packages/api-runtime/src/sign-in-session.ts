import { decodeUnverifiedJwtPayload } from './unverified-display-identity.js';

/** The signed-in user, named by the token's `sub`. */
export interface SignInUser {
  readonly id: string;
}

/** The active organization from Clerk's `o` claim or WorkOS's `org_id`, `role` and `permissions`. */
export interface SignInOrganization {
  readonly id: string;
  readonly role?: string;
  readonly slug?: string;
  readonly permissions: readonly string[];
}

/** One session shape for every sign-in adapter. */
export interface SignInSession {
  readonly sessionId?: string;
  readonly user: SignInUser;
  readonly organization?: SignInOrganization;
}

type Claims = Readonly<Record<string, unknown>>;

/**
 * Reads the session shape from unverified access-token claims.
 *
 * The API verifies the token. Use this result for display and routing only,
 * never for authorization.
 */
export function unverifiedSignInSessionFromJwt(token: string): SignInSession | undefined {
  const claims = decodeUnverifiedJwtPayload(token);
  const subject = claims === undefined ? undefined : text(claims, 'sub');
  if (claims === undefined || subject === undefined) return undefined;
  const sessionId = text(claims, 'sid');
  const organization = clerkOrganization(claims) ?? workosOrganization(claims);
  return {
    ...(sessionId === undefined ? {} : { sessionId }),
    user: { id: subject },
    ...(organization === undefined ? {} : { organization }),
  };
}

function clerkOrganization(claims: Claims): SignInOrganization | undefined {
  const value = claims['o'];
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return undefined;
  const claim = value as Claims;
  const id = text(claim, 'id');
  if (id === undefined) return undefined;
  const role = text(claim, 'rol');
  const slug = text(claim, 'slg');
  const permissions = text(claim, 'per');
  return {
    id,
    ...(role === undefined ? {} : { role }),
    ...(slug === undefined ? {} : { slug }),
    permissions: permissions === undefined ? [] : permissions.split(',').filter(Boolean),
  };
}

function workosOrganization(claims: Claims): SignInOrganization | undefined {
  const id = text(claims, 'org_id');
  if (id === undefined) return undefined;
  const role = text(claims, 'role');
  const permissions = claims['permissions'];
  return {
    id,
    ...(role === undefined ? {} : { role }),
    permissions: Array.isArray(permissions)
      ? permissions.filter((permission): permission is string => typeof permission === 'string')
      : [],
  };
}

function text(claims: Claims, name: string): string | undefined {
  const value = claims[name];
  return typeof value === 'string' && value !== '' ? value : undefined;
}
