import {
  SuiteClient,
  suiteErrorCode,
  type AuthorizationInput,
  type AuthorizationPreview,
} from './api.js';
export type AuthorizeState =
  | { readonly type: 'loading' | 'login' | 'framed' | 'web_required' | 'invalid' | 'submitting' }
  | { readonly type: 'consent' | 'hint_mismatch'; readonly preview: AuthorizationPreview }
  | { readonly type: 'redirect'; readonly url: string }
  | { readonly type: 'failed'; readonly code: string };
export interface AuthorizeEnvironment {
  readonly web: boolean;
  readonly framed: boolean;
  readonly signedIn: boolean;
}
/** Products render this state and supply their own login and account-switch UI. */
export class SuiteAuthorizeMachine {
  #state: AuthorizeState = { type: 'loading' };
  #preview: AuthorizationPreview | undefined;
  #started = false;
  public constructor(
    private readonly client: SuiteClient,
    private readonly input: AuthorizationInput & { readonly hintDomain?: string },
    private readonly environment: AuthorizeEnvironment,
  ) {}
  public get state(): AuthorizeState {
    return this.#state;
  }
  public async load(): Promise<AuthorizeState> {
    if (this.#started) return this.#state;
    this.#started = true;
    if (this.environment.framed) return this.set({ type: 'framed' });
    if (!this.environment.web) return this.set({ type: 'web_required' });
    if (!this.input.client || !this.input.state || !this.input.codeChallenge)
      return this.set({ type: 'invalid' });
    if (!this.environment.signedIn) return this.set({ type: 'login' });
    try {
      this.#preview = await this.client.preview(
        this.input.client,
        this.input.hint,
        this.input.hintDomain,
      );
      if (this.#preview.hintMismatch)
        return this.set({ type: 'hint_mismatch', preview: this.#preview });
      if (this.#preview.autoApprove) return await this.approve();
      return this.set({ type: 'consent', preview: this.#preview });
    } catch (error) {
      return this.set({ type: 'failed', code: suiteErrorCode(error) });
    }
  }
  public async approve(): Promise<AuthorizeState> {
    if (
      this.environment.framed ||
      !this.environment.web ||
      !this.environment.signedIn ||
      this.#preview === undefined ||
      this.#preview.hintMismatch ||
      this.#state.type === 'submitting' ||
      this.#state.type === 'redirect'
    )
      return this.#state;
    this.set({ type: 'submitting' });
    try {
      const { client, state, codeChallenge, hint } = this.input;
      const result = await this.client.authorize({
        client,
        state,
        codeChallenge,
        ...(hint === undefined ? {} : { hint }),
      });
      return this.set({ type: 'redirect', url: result.redirectUrl });
    } catch (error) {
      return this.set({ type: 'failed', code: suiteErrorCode(error) });
    }
  }
  public async deny(): Promise<AuthorizeState> {
    if (
      !['consent', 'hint_mismatch', 'failed'].includes(this.#state.type) ||
      this.environment.framed
    )
      return this.#state;
    this.set({ type: 'submitting' });
    try {
      const result = await this.client.deny({ client: this.input.client, state: this.input.state });
      return this.set({ type: 'redirect', url: result.redirectUrl });
    } catch (error) {
      return this.set({ type: 'failed', code: suiteErrorCode(error) });
    }
  }
  private set(state: AuthorizeState): AuthorizeState {
    this.#state = state;
    return state;
  }
}

/** Rejects duplicate protocol values before any preview or authorization request. */
export function parseSuiteAuthorizeQuery(
  query: URLSearchParams,
): (AuthorizationInput & { readonly hintDomain?: string }) | null {
  for (const key of ['client', 'state', 'code_challenge', 'hint', 'hint_domain']) {
    if (query.getAll(key).length > 1) return null;
  }
  const client = query.get('client');
  const state = query.get('state');
  const codeChallenge = query.get('code_challenge');
  if (!client || !state || !codeChallenge) return null;
  const hint = query.get('hint');
  const hintDomain = query.get('hint_domain');
  return {
    client,
    state,
    codeChallenge,
    ...(hint === null ? {} : { hint }),
    ...(hintDomain === null ? {} : { hintDomain }),
  };
}
export function suiteAuthorizationReturnPath(
  input: AuthorizationInput & { readonly hintDomain?: string },
): string {
  const query = new URLSearchParams({
    client: input.client,
    state: input.state,
    code_challenge: input.codeChallenge,
  });
  if (input.hint !== undefined) query.set('hint', input.hint);
  if (input.hintDomain !== undefined) query.set('hint_domain', input.hintDomain);
  return `/suite/authorize?${query.toString()}`;
}
/** Logout must finish before login starts, so auto-approval cannot use the old session. */
export async function switchSuiteAccount(
  input: AuthorizationInput & { readonly hintDomain?: string },
  auth: {
    readonly logout: (returnPath: string) => Promise<void>;
    readonly login: (returnPath: string) => Promise<void>;
  },
): Promise<void> {
  const path = suiteAuthorizationReturnPath(input);
  await auth.logout(path);
  await auth.login(path);
}
