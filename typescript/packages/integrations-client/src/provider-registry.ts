import type { ConnectionAction, ConnectionState, ConnectionStatus } from './connection-health.js';

export interface ProviderRegistration<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
  Connector = undefined,
> {
  readonly id: ProviderId;
  readonly labelKey: LabelKey;
  readonly capabilities: readonly Capability[];
  readonly connector?: Connector;
  readonly connection?: ConnectionState;
}

export interface RegisteredProvider<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
  Connector = undefined,
> {
  readonly id: ProviderId;
  readonly labelKey: LabelKey;
  readonly capabilities: readonly Capability[];
  readonly connector?: Connector;
  readonly currentState: ConnectionStatus;
  readonly availableActions: readonly ConnectionAction[];
}

export type ProviderConnectionStates<ProviderId extends string> =
  ReadonlyMap<ProviderId, ConnectionState> | Readonly<Partial<Record<ProviderId, ConnectionState>>>;

type ProviderDefinition<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
  Connector,
> = Omit<ProviderRegistration<ProviderId, LabelKey, Capability, Connector>, 'connection'>;

export class ProviderRegistry<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
  Connector = undefined,
> {
  readonly #definitions: readonly ProviderDefinition<ProviderId, LabelKey, Capability, Connector>[];
  readonly #providers: readonly RegisteredProvider<ProviderId, LabelKey, Capability, Connector>[];
  readonly #byId: ReadonlyMap<
    string,
    RegisteredProvider<ProviderId, LabelKey, Capability, Connector>
  >;

  public constructor(
    registrations: readonly ProviderRegistration<ProviderId, LabelKey, Capability, Connector>[],
  ) {
    const definitions = registrations.map((registration) =>
      Object.freeze({
        id: registration.id,
        labelKey: registration.labelKey,
        capabilities: Object.freeze([...registration.capabilities]),
        ...('connector' in registration ? { connector: registration.connector } : {}),
      }),
    );
    const ids = new Set(definitions.map((definition) => definition.id));
    if (ids.size !== definitions.length) throw new TypeError('Provider IDs must be unique.');

    const providers = definitions.map((definition, index) => {
      const connection = registrations[index]?.connection;
      return Object.freeze({
        ...definition,
        currentState: connection?.status ?? 'disconnected',
        availableActions: Object.freeze(
          connection === undefined ? [] : [...connection.availableActions],
        ),
      });
    });
    this.#definitions = Object.freeze(definitions);
    this.#providers = Object.freeze(providers);
    this.#byId = new Map(providers.map((provider) => [provider.id, provider]));
  }

  public list(): readonly RegisteredProvider<ProviderId, LabelKey, Capability, Connector>[] {
    return this.#providers;
  }

  public get(
    providerId: string,
  ): RegisteredProvider<ProviderId, LabelKey, Capability, Connector> | undefined {
    return this.#byId.get(providerId);
  }

  public withConnectionStates(
    states: ProviderConnectionStates<ProviderId>,
  ): ProviderRegistry<ProviderId, LabelKey, Capability, Connector> {
    const registrations = this.#definitions.map((definition) => {
      const connection = getConnectionState(states, definition.id);
      return {
        ...definition,
        ...(connection === undefined ? {} : { connection }),
      };
    });
    return new ProviderRegistry<ProviderId, LabelKey, Capability, Connector>(registrations);
  }
}

export function createProviderRegistry<
  ProviderId extends string,
  LabelKey extends string,
  Capability extends string,
  Connector = undefined,
>(
  registrations: readonly ProviderRegistration<ProviderId, LabelKey, Capability, Connector>[],
): ProviderRegistry<ProviderId, LabelKey, Capability, Connector> {
  return new ProviderRegistry(registrations);
}

function getConnectionState<ProviderId extends string>(
  states: ProviderConnectionStates<ProviderId>,
  providerId: ProviderId,
): ConnectionState | undefined {
  if (isReadonlyMap(states)) return states.get(providerId);
  if (!Object.hasOwn(states, providerId)) return undefined;
  return states[providerId];
}

function isReadonlyMap<ProviderId extends string>(
  states: ProviderConnectionStates<ProviderId>,
): states is ReadonlyMap<ProviderId, ConnectionState> {
  return typeof Reflect.get(states, 'get') === 'function';
}
