import {
  OAuthSessionCoordinator,
  type OAuthSessionCoordinatorOptions,
  type OAuthSessionOutcome,
} from '@baukit/integrations-client';
import { SuiteClient, SuiteFlowError, type SuiteLink } from './api.js';
import { suiteReturnUrl, suiteReturnValidator, type SuiteReturnPolicy } from './urls.js';

export type SuiteConnectionResult =
  | { readonly type: 'connected'; readonly requestId: string; readonly link: SuiteLink }
  | { readonly type: 'denied' }
  | { readonly type: 'cancelled' };
export interface SuiteSessionOptions extends SuiteReturnPolicy {
  readonly client: SuiteClient;
  readonly coordinator: Omit<OAuthSessionCoordinatorOptions, 'validateReturnUrl' | 'timeoutMs'>;
}
/** One instance per signed-in session. Share it between native and redirect handlers. */
export class SuiteSession {
  readonly #coordinator: OAuthSessionCoordinator;
  #nativeConnection: Promise<SuiteConnectionResult> | undefined;
  #redirect: { readonly url: string; readonly result: Promise<SuiteConnectionResult> } | undefined;
  #nativeReturn: string | undefined;
  public constructor(private readonly options: SuiteSessionOptions) {
    const native = options.coordinator.native;
    this.#coordinator = new OAuthSessionCoordinator({
      ...options.coordinator,
      ...(native === undefined
        ? {}
        : {
            native: {
              run: async (input) => {
                const result = await native.run(input);
                if (result.type === 'success') this.#nativeReturn = result.returnUrl;
                return result;
              },
              cancel: () => {
                native.cancel();
              },
            },
          }),
      timeoutMs: 600_000,
      validateReturnUrl: suiteReturnValidator(options),
    });
  }
  public connect(peerApp: string): Promise<SuiteConnectionResult> {
    if (this.#nativeConnection !== undefined) return this.#nativeConnection;
    this.#redirect = undefined;
    this.#nativeReturn = undefined;
    const operation = this.#coordinator
      .authorize({
        platform: this.options.platform,
        returnUrl: suiteReturnUrl(this.options),
        createAuthorizationUrl: async ({ returnUrl, stateNonce }) =>
          (await this.options.client.start({ peerApp, returnUrl, stateNonce })).authorizeUrl,
      })
      .then((outcome) => this.complete(outcome));
    if (this.options.platform === 'web') return operation;
    const result = operation.finally(() => {
      if (this.#nativeReturn !== undefined) this.#redirect = { url: this.#nativeReturn, result };
      if (this.#nativeConnection === result) this.#nativeConnection = undefined;
    });
    this.#nativeConnection = result;
    return result;
  }
  public handleRedirect(url: string): Promise<SuiteConnectionResult> {
    if (this.#nativeConnection !== undefined) return this.#nativeConnection;
    if (this.#redirect?.url === url) return this.#redirect.result;
    const result = this.#coordinator.handleRedirect(url).then((outcome) => this.complete(outcome));
    this.#redirect = { url, result };
    return result;
  }
  private async complete(outcome: OAuthSessionOutcome): Promise<SuiteConnectionResult> {
    if (outcome.type === 'cancelled') return { type: 'cancelled' };
    if (outcome.type !== 'success') throw new SuiteFlowError('suite_code_invalid');
    const params = outcome.callbackParams;
    if (params['status'] === 'denied') return { type: 'denied' };
    if (params['status'] === 'failed')
      throw new SuiteFlowError(params['code'] ?? 'suite_code_invalid');
    const requestId = params['request'];
    const code = params['code'];
    if (!requestId || !code || params['status']) throw new SuiteFlowError('suite_code_invalid');
    return {
      type: 'connected',
      requestId,
      link: await this.options.client.complete(requestId, code),
    };
  }
}
