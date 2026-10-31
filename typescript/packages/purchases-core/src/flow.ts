import type {
  CreditBalanceSnapshot,
  EntitlementBackend,
  VerifiedEntitlement,
} from './entitlement.js';
import { isPurchaseError, PurchaseError } from './errors.js';
import type { PurchaseOfferings, PurchasePackage, PurchasesPort } from './port.js';

export const DEFAULT_REFRESH_ATTEMPTS = 5;
export const DEFAULT_REFRESH_INTERVAL_MS = 2_000;
const NO_REVISION = -1;

export interface PurchaseState {
  readonly suiteSubject: string | null;
  readonly entitlement: VerifiedEntitlement | null;
  readonly creditBalance: CreditBalanceSnapshot | null;
}

export type PurchaseOutcome =
  | { readonly status: 'granted'; readonly entitlement: VerifiedEntitlement }
  | { readonly status: 'pending'; readonly entitlement: VerifiedEntitlement | null };

export type RestoreOutcome =
  | { readonly status: 'granted'; readonly entitlement: VerifiedEntitlement }
  | { readonly status: 'unchanged'; readonly entitlement: VerifiedEntitlement | null };

export type CreditPackOutcome =
  | { readonly status: 'credited'; readonly balance: CreditBalanceSnapshot }
  | { readonly status: 'pending'; readonly balance: CreditBalanceSnapshot | null };

export interface PurchaseFlowOptions {
  readonly port: PurchasesPort;
  readonly backend: EntitlementBackend;
  /** Backend reads after a purchase before the outcome is reported as pending. */
  readonly refreshAttempts?: number;
  readonly refreshIntervalMs?: number;
  readonly sleep?: (milliseconds: number) => Promise<void>;
}

interface Identity {
  readonly generation: number;
  readonly suiteSubject: string;
}

interface Revisioned {
  readonly revision: number;
}

const SIGNED_OUT: PurchaseState = { suiteSubject: null, entitlement: null, creditBalance: null };

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function storeError(error: unknown): PurchaseError {
  return isPurchaseError(error) ? error : new PurchaseError('store', { cause: error });
}

export class PurchaseFlow {
  readonly #port: PurchasesPort;
  readonly #backend: EntitlementBackend;
  readonly #attempts: number;
  readonly #interval: number;
  readonly #sleep: (milliseconds: number) => Promise<void>;
  readonly #listeners = new Set<() => void>();
  readonly #stopCustomerUpdates: () => void;
  #state: PurchaseState = SIGNED_OUT;
  #generation = 0;
  #busy = false;

  constructor(options: PurchaseFlowOptions) {
    this.#port = options.port;
    this.#backend = options.backend;
    this.#attempts = Math.max(1, options.refreshAttempts ?? DEFAULT_REFRESH_ATTEMPTS);
    this.#interval = options.refreshIntervalMs ?? DEFAULT_REFRESH_INTERVAL_MS;
    this.#sleep = options.sleep ?? wait;
    this.#stopCustomerUpdates = this.#port.onCustomerUpdate(() => {
      this.#refreshQuietly();
    });
  }

  readonly getState = (): PurchaseState => this.#state;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /** Purchases stay refused until the store has finished identifying this suite subject. */
  async identify(suiteSubject: string): Promise<void> {
    if (suiteSubject.trim() === '') throw new PurchaseError('invalid_subject');
    const generation = this.#restart();
    await this.#port.identify(suiteSubject);
    this.#assertGeneration(generation);
    this.#setState({ ...SIGNED_OUT, suiteSubject });
  }

  async signOut(): Promise<void> {
    this.#restart();
    await this.#port.reset();
  }

  offerings(): Promise<PurchaseOfferings> {
    return this.#port.offerings();
  }

  async refreshEntitlement(): Promise<VerifiedEntitlement | null> {
    const identity = this.#identity();
    const entitlement = await this.#backend.refreshEntitlement();
    this.#accept(identity, entitlement, 'entitlement');
    return this.#state.entitlement;
  }

  async refreshCreditBalance(): Promise<CreditBalanceSnapshot | null> {
    const identity = this.#identity();
    const read = this.#backend.creditBalance?.bind(this.#backend);
    if (read === undefined) throw new PurchaseError('unavailable');
    const balance = await read();
    this.#accept(identity, balance, 'creditBalance');
    return this.#state.creditBalance;
  }

  purchase(item: PurchasePackage): Promise<PurchaseOutcome> {
    return this.#exclusive(async (identity) => {
      const baseline = await this.#entitlementBaseline();
      await this.#storeCall(identity, () => this.#port.purchase(item));
      const entitlement = await this.#poll(identity, baseline, () => this.refreshEntitlement());
      return entitlement
        ? { status: 'granted', entitlement }
        : { status: 'pending', entitlement: this.#state.entitlement };
    });
  }

  restore(): Promise<RestoreOutcome> {
    return this.#exclusive(async (identity) => {
      const baseline = await this.#entitlementBaseline();
      await this.#storeCall(identity, () => this.#port.restore());
      const entitlement = await this.#poll(identity, baseline, () => this.refreshEntitlement());
      return entitlement
        ? { status: 'granted', entitlement }
        : { status: 'unchanged', entitlement: this.#state.entitlement };
    });
  }

  purchaseCreditPack(item: PurchasePackage): Promise<CreditPackOutcome> {
    if (item.kind !== 'one_time') return Promise.reject(new PurchaseError('not_credit_pack'));
    return this.#exclusive(async (identity) => {
      const baseline = (await this.refreshCreditBalance())?.revision;
      await this.#storeCall(identity, () => this.#port.purchase(item));
      const balance = await this.#poll(identity, baseline ?? NO_REVISION, () =>
        this.refreshCreditBalance(),
      );
      return balance
        ? { status: 'credited', balance }
        : { status: 'pending', balance: this.#state.creditBalance };
    });
  }

  dispose(): void {
    this.#stopCustomerUpdates();
    this.#listeners.clear();
  }

  async #entitlementBaseline(): Promise<number> {
    const entitlement = await this.refreshEntitlement();
    return entitlement?.revision ?? NO_REVISION;
  }

  async #storeCall(identity: Identity, call: () => Promise<void>): Promise<void> {
    try {
      await call();
    } catch (error) {
      throw storeError(error);
    }
    this.#assertGeneration(identity.generation);
  }

  async #poll<T extends Revisioned>(
    identity: Identity,
    baseline: number,
    read: () => Promise<T | null>,
  ): Promise<T | null> {
    for (let attempt = 0; attempt < this.#attempts; attempt += 1) {
      if (attempt > 0) await this.#sleep(this.#interval);
      this.#assertGeneration(identity.generation);
      const value = await this.#readTolerantly(read);
      if (value !== null && value.revision > baseline) return value;
    }
    return null;
  }

  async #readTolerantly<T>(read: () => Promise<T | null>): Promise<T | null> {
    try {
      return await read();
    } catch (error) {
      if (isPurchaseError(error)) throw error;
      return null;
    }
  }

  async #exclusive<T>(operation: (identity: Identity) => Promise<T>): Promise<T> {
    const identity = this.#identity();
    if (this.#busy) throw new PurchaseError('busy');
    this.#busy = true;
    try {
      return await operation(identity);
    } finally {
      this.#busy = false;
    }
  }

  #accept<K extends 'entitlement' | 'creditBalance'>(
    identity: Identity,
    value: PurchaseState[K],
    key: K,
  ): void {
    this.#assertGeneration(identity.generation);
    if (value === null) return;
    if (value.suiteSubject !== identity.suiteSubject) throw new PurchaseError('subject_mismatch');
    const current = this.#state[key];
    if (current !== null && value.revision < current.revision) return;
    this.#setState({ ...this.#state, [key]: value });
  }

  #identity(): Identity {
    const { suiteSubject } = this.#state;
    if (suiteSubject === null) throw new PurchaseError('not_identified');
    return { generation: this.#generation, suiteSubject };
  }

  #restart(): number {
    this.#generation += 1;
    this.#setState(SIGNED_OUT);
    return this.#generation;
  }

  #assertGeneration(generation: number): void {
    if (generation !== this.#generation) throw new PurchaseError('identity_changed');
  }

  #refreshQuietly(): void {
    if (this.#state.suiteSubject === null) return;
    this.refreshEntitlement().catch(() => undefined);
  }

  #setState(state: PurchaseState): void {
    this.#state = state;
    for (const listener of this.#listeners) listener();
  }
}
