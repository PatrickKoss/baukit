import {
  isAuthorizationPreview,
  isSuiteDeliveries,
  isSuiteLink,
  isSuiteLinks,
  isSuiteLinkStart,
  isSuitePeers,
  isSuiteRedirect,
  parseSuiteResponse,
  type SuiteLinkStart,
  type SuiteRedirect,
} from './responses.js';
export type RewardMode = 'native' | 'source_xp' | 'off';
export type DeliveryHealth = 'healthy' | 'degraded' | 'needs_attention' | 'disabled';
export interface SuiteLink {
  readonly id: string;
  readonly peerApp: string;
  readonly role: 'initiator' | 'authorizer';
  readonly remoteLinkId: string;
  readonly remoteDisplayName: string | null;
  readonly status: 'active' | 'needs_attention' | 'revoked';
  readonly sends: readonly string[];
  readonly receives: readonly string[];
  readonly shareXp: boolean;
  readonly rewardMode: RewardMode;
  readonly deliveryHealth: DeliveryHealth;
  readonly consecutiveFailures: number;
  readonly lastDeliveryAt: string | null;
  readonly lastFailureAt: string | null;
  readonly lastFailureCode: string | null;
  readonly lastReceivedAt: string | null;
  readonly replayEarliest: string;
  readonly createdAt: string;
  readonly updatedAt: string;
}
export interface SuitePeer {
  readonly id: string;
  readonly displayName: string;
  readonly scheme: string;
  readonly webUrl: string;
  readonly shareXpAvailable: boolean;
  readonly sends: readonly string[];
  readonly receives: readonly string[];
  readonly rewardModes: readonly RewardMode[];
  readonly link: string | null;
}
export interface AuthorizationPreview {
  readonly peer: string;
  readonly authorizerSends: readonly string[];
  readonly initiatorSends: readonly string[];
  readonly existingLink: boolean;
  readonly autoApprove: boolean;
  readonly hintMismatch: boolean;
}
export interface AuthorizationInput {
  readonly client: string;
  readonly state: string;
  readonly codeChallenge: string;
  readonly hint?: string;
}
export interface SuiteDelivery {
  readonly jobId: string;
  readonly linkId: string;
  readonly eventType: string | null;
  readonly status: 'pending' | 'running' | 'succeeded' | 'failed' | 'cancelled';
  readonly attemptCount: number;
  readonly lastFailureCode: string | null;
  readonly createdAt: string;
  readonly updatedAt: string;
}
/**
 * Supply the product's authenticated, JSON-decoding HTTP transport. Return the decoded body
 * untyped; SuiteClient validates every response and throws SuiteResponseError on a mismatch.
 */
export interface SuiteTransport {
  request(
    method: 'GET' | 'POST' | 'PATCH' | 'DELETE',
    path: string,
    body?: unknown,
  ): Promise<unknown>;
}
export class SuiteFlowError extends Error {
  public constructor(public readonly code: string) {
    super(code);
  }
}
export type SuiteJsonValue =
  | null
  | boolean
  | number
  | string
  | readonly SuiteJsonValue[]
  | { readonly [key: string]: SuiteJsonValue };
export type SuiteErrorDetails = Readonly<Record<string, SuiteJsonValue>>;

export function suiteErrorDetails(error: unknown): SuiteErrorDetails | undefined {
  if (!(error instanceof Error) || !('details' in error)) return undefined;
  const details: unknown = error.details;
  return isJsonObject(details) ? details : undefined;
}
function isJsonObject(value: unknown): value is SuiteErrorDetails {
  return (
    typeof value === 'object' &&
    value !== null &&
    !Array.isArray(value) &&
    Object.values(value).every(isJsonValue)
  );
}
function isJsonValue(value: unknown): value is SuiteJsonValue {
  return (
    value === null ||
    typeof value === 'string' ||
    typeof value === 'boolean' ||
    (typeof value === 'number' && Number.isFinite(value)) ||
    (Array.isArray(value) ? value.every(isJsonValue) : isJsonObject(value))
  );
}

export function suiteErrorCode(error: unknown): string {
  if (error instanceof Error && 'code' in error && typeof error.code === 'string')
    return error.code;
  return 'suite_unavailable';
}
const linkPath = (id: string) => `/suite/links/${encodeURIComponent(id)}`;
export class SuiteClient {
  public constructor(private readonly transport: SuiteTransport) {}
  private async call<T>(
    guard: (value: unknown) => value is T,
    method: 'GET' | 'POST' | 'PATCH' | 'DELETE',
    path: string,
    body?: unknown,
  ): Promise<T> {
    const response = await this.transport.request(method, path, body);
    return parseSuiteResponse(`${method} ${path.split('?')[0] ?? path}`, response, guard);
  }
  private async send(method: 'POST' | 'DELETE', path: string, body?: unknown): Promise<void> {
    await this.transport.request(method, path, body);
  }
  public peers(): Promise<readonly SuitePeer[]> {
    return this.call(isSuitePeers, 'GET', '/suite/peers');
  }
  public list(): Promise<readonly SuiteLink[]> {
    return this.call(isSuiteLinks, 'GET', '/suite/links');
  }
  public get(id: string): Promise<SuiteLink> {
    return this.call(isSuiteLink, 'GET', linkPath(id));
  }
  public start(input: {
    readonly peerApp: string;
    readonly returnUrl: string;
    readonly stateNonce: string;
  }): Promise<SuiteLinkStart> {
    return this.call(isSuiteLinkStart, 'POST', '/suite/links', input);
  }
  public complete(requestId: string, code: string): Promise<SuiteLink> {
    return this.call(
      isSuiteLink,
      'POST',
      `/suite/links/requests/${encodeURIComponent(requestId)}/complete`,
      { code },
    );
  }
  public preview(
    client: string,
    hint?: string,
    hintDomain?: string,
  ): Promise<AuthorizationPreview> {
    const query = new URLSearchParams({ client });
    if (hint !== undefined) query.set('hint', hint);
    if (hintDomain !== undefined) query.set('hint_domain', hintDomain);
    return this.call(
      isAuthorizationPreview,
      'GET',
      `/suite/authorizations/preview?${query.toString()}`,
    );
  }
  public authorize(input: AuthorizationInput): Promise<SuiteRedirect> {
    return this.call(isSuiteRedirect, 'POST', '/suite/authorizations', input);
  }
  public deny(input: { readonly client: string; readonly state: string }): Promise<SuiteRedirect> {
    return this.call(isSuiteRedirect, 'POST', '/suite/authorizations/deny', input);
  }
  public update(
    id: string,
    input: { readonly shareXp?: boolean; readonly rewardMode?: RewardMode },
  ): Promise<SuiteLink> {
    return this.call(isSuiteLink, 'PATCH', linkPath(id), input);
  }
  public disconnect(id: string): Promise<void> {
    return this.send('DELETE', linkPath(id));
  }
  public test(id: string): Promise<void> {
    return this.send('POST', `${linkPath(id)}/test`);
  }
  public reenable(id: string): Promise<SuiteLink> {
    return this.call(isSuiteLink, 'POST', `${linkPath(id)}/reenable`);
  }
  public replay(id: string, since: string): Promise<void> {
    return this.send('POST', `${linkPath(id)}/replay`, { since });
  }
  public deliveries(id: string): Promise<readonly SuiteDelivery[]> {
    return this.call(isSuiteDeliveries, 'GET', `${linkPath(id)}/deliveries`);
  }
}
