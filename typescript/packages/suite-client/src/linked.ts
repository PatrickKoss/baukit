import { suiteErrorCode } from './api.js';
import { SuiteSession } from './session.js';
import { type SuiteNavigationStore } from './navigation.js';
export type LinkedState =
  | { readonly type: 'restoring' | 'completing' }
  | { readonly type: 'login'; readonly returnPath: string }
  | { readonly type: 'connected_apps'; readonly noticeToken?: string };
export interface SuiteLinkedOptions {
  readonly originalUrl: string;
  readonly session: SuiteSession;
  readonly navigation: SuiteNavigationStore;
  readonly scrubHistory: (path: string) => void;
  readonly refresh: () => Promise<void>;
}
/** Retains the callback in memory while removing codes from browser history. */
export class SuiteLinkedMachine {
  #state: LinkedState = { type: 'restoring' };
  #operation: Promise<LinkedState> | undefined;
  public constructor(private readonly options: SuiteLinkedOptions) {
    options.scrubHistory('/suite/linked');
  }
  public get state(): LinkedState {
    return this.#state;
  }
  public async restore(restoring: boolean, signedIn: boolean): Promise<LinkedState> {
    if (restoring) return this.#state;
    if (!signedIn) {
      this.#state = {
        type: 'login',
        returnPath: `/suite/linked${new URL(this.options.originalUrl).search}`,
      };
      return this.#state;
    }
    this.#operation ??= this.complete();
    return this.#operation;
  }
  private async complete(): Promise<LinkedState> {
    this.#state = { type: 'completing' };
    try {
      const result = await this.options.session.handleRedirect(this.options.originalUrl);
      await this.options.refresh();
      this.#state = {
        type: 'connected_apps',
        ...(result.type === 'connected'
          ? { noticeToken: this.options.navigation.createSuiteConnectionNotice(result.requestId) }
          : {}),
      };
    } catch (error) {
      this.#state = {
        type: 'connected_apps',
        noticeToken: this.options.navigation.createSuiteErrorNotice(suiteErrorCode(error)),
      };
    }
    return this.#state;
  }
}
