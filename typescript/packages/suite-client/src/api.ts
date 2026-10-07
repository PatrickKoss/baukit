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
/** Supply the product's authenticated, JSON-decoding HTTP transport. */
export interface SuiteTransport {
  request<T>(method: 'GET' | 'POST' | 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<T>;
}
export class SuiteFlowError extends Error {
  public constructor(public readonly code: string) {
    super(code);
  }
}
export function suiteErrorCode(error: unknown): string {
  if (error instanceof Error && 'code' in error && typeof error.code === 'string')
    return error.code;
  return 'suite_unavailable';
}
const linkPath = (id: string) => `/suite/links/${encodeURIComponent(id)}`;
export class SuiteClient {
  public constructor(private readonly transport: SuiteTransport) {}
  public peers(): Promise<readonly SuitePeer[]> {
    return this.transport.request('GET', '/suite/peers');
  }
  public list(): Promise<readonly SuiteLink[]> {
    return this.transport.request('GET', '/suite/links');
  }
  public get(id: string): Promise<SuiteLink> {
    return this.transport.request('GET', linkPath(id));
  }
  public start(input: {
    readonly peerApp: string;
    readonly returnUrl: string;
    readonly stateNonce: string;
  }): Promise<{ readonly requestId: string; readonly authorizeUrl: string }> {
    return this.transport.request('POST', '/suite/links', input);
  }
  public complete(requestId: string, code: string): Promise<SuiteLink> {
    return this.transport.request(
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
    return this.transport.request('GET', `/suite/authorizations/preview?${query.toString()}`);
  }
  public authorize(input: AuthorizationInput): Promise<{ readonly redirectUrl: string }> {
    return this.transport.request('POST', '/suite/authorizations', input);
  }
  public deny(input: {
    readonly client: string;
    readonly state: string;
  }): Promise<{ readonly redirectUrl: string }> {
    return this.transport.request('POST', '/suite/authorizations/deny', input);
  }
  public update(
    id: string,
    input: { readonly shareXp?: boolean; readonly rewardMode?: RewardMode },
  ): Promise<SuiteLink> {
    return this.transport.request('PATCH', linkPath(id), input);
  }
  public disconnect(id: string): Promise<void> {
    return this.transport.request('DELETE', linkPath(id));
  }
  public test(id: string): Promise<void> {
    return this.transport.request('POST', `${linkPath(id)}/test`);
  }
  public reenable(id: string): Promise<SuiteLink> {
    return this.transport.request('POST', `${linkPath(id)}/reenable`);
  }
  public replay(id: string, since: string): Promise<void> {
    return this.transport.request('POST', `${linkPath(id)}/replay`, { since });
  }
  public deliveries(id: string): Promise<readonly SuiteDelivery[]> {
    return this.transport.request('GET', `${linkPath(id)}/deliveries`);
  }
}
