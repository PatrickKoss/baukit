import type { ConnectionAction, ConnectionState, ConnectionStatus } from './connection-health.js';

export interface ProviderRegistration<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
> {
  readonly id: ProviderId;
  readonly labelKey: LabelKey;
  readonly capabilities: readonly Capability[];
  readonly connection: ConnectionState;
}

export interface RegisteredProvider<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
> {
  readonly id: ProviderId;
  readonly labelKey: LabelKey;
  readonly capabilities: readonly Capability[];
  readonly currentState: ConnectionStatus;
  readonly availableActions: readonly ConnectionAction[];
}

export class ProviderRegistry<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
> {
  readonly #providers: readonly RegisteredProvider<ProviderId, LabelKey, Capability>[];
  readonly #byId: ReadonlyMap<ProviderId, RegisteredProvider<ProviderId, LabelKey, Capability>>;

  public constructor(
    registrations: readonly ProviderRegistration<ProviderId, LabelKey, Capability>[],
  ) {
    const providers = registrations.map((registration) =>
      Object.freeze({
        id: registration.id,
        labelKey: registration.labelKey,
        capabilities: Object.freeze([...registration.capabilities]),
        currentState: registration.connection.status,
        availableActions: Object.freeze([...registration.connection.availableActions]),
      }),
    );
    const byId = new Map(providers.map((provider) => [provider.id, provider]));
    if (byId.size !== providers.length) throw new TypeError('Provider IDs must be unique.');
    this.#providers = Object.freeze(providers);
    this.#byId = byId;
  }

  public list(): readonly RegisteredProvider<ProviderId, LabelKey, Capability>[] {
    return this.#providers;
  }

  public get(
    providerId: ProviderId,
  ): RegisteredProvider<ProviderId, LabelKey, Capability> | undefined {
    return this.#byId.get(providerId);
  }
}

export function createProviderRegistry<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
>(
  registrations: readonly ProviderRegistration<ProviderId, LabelKey, Capability>[],
): ProviderRegistry<ProviderId, LabelKey, Capability> {
  return new ProviderRegistry(registrations);
}
