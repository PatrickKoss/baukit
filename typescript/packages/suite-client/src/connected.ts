import { SuiteClient, suiteErrorCode, type SuiteLink, type SuitePeer } from './api.js';
import { SuiteSession } from './session.js';
export type ConnectedAppsState =
  | { readonly type: 'loading' }
  | {
      readonly type: 'ready';
      readonly peers: readonly SuitePeer[];
      readonly links: readonly SuiteLink[];
    }
  | { readonly type: 'connecting'; readonly peerApp: string }
  | { readonly type: 'denied' | 'cancelled' }
  | { readonly type: 'failed'; readonly code: string };
export function suiteConnectionState(
  link: SuiteLink | undefined,
): 'not_connected' | 'connected' | 'needs_attention' | 'disabled' {
  if (link === undefined || link.status === 'revoked') return 'not_connected';
  if (link.status === 'needs_attention') return 'needs_attention';
  if (link.deliveryHealth === 'needs_attention' || link.deliveryHealth === 'disabled')
    return link.deliveryHealth;
  return 'connected';
}
export class ConnectedApps {
  #state: ConnectedAppsState = { type: 'loading' };
  public constructor(
    private readonly client: SuiteClient,
    private readonly session: SuiteSession,
  ) {}
  public get state(): ConnectedAppsState {
    return this.#state;
  }
  public async load(): Promise<ConnectedAppsState> {
    this.#state = { type: 'loading' };
    try {
      const [peers, links] = await Promise.all([this.client.peers(), this.client.list()]);
      this.#state = { type: 'ready', peers, links };
    } catch (error) {
      this.#state = { type: 'failed', code: suiteErrorCode(error) };
    }
    return this.#state;
  }
  /** Connecting an already linked peer uses the same flow and replaces both links. */
  public async connect(peerApp: string): Promise<ConnectedAppsState> {
    if (this.#state.type === 'connecting') return this.#state;
    this.#state = { type: 'connecting', peerApp };
    try {
      const result = await this.session.connect(peerApp);
      if (result.type === 'connected') return await this.load();
      this.#state = { type: result.type };
    } catch (error) {
      this.#state = { type: 'failed', code: suiteErrorCode(error) };
    }
    return this.#state;
  }
}
