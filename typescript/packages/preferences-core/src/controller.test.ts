import { describe, expect, it, vi } from 'vitest';

import {
  createPreferenceController,
  type PreferenceRegistry,
  type PreferenceSideEffect,
} from './index.js';
import { InMemoryPreferenceStore, type PreferenceStore } from './store.js';

interface TestPreferences {
  readonly language: 'system' | 'de' | 'en';
  readonly theme: 'system' | 'dark' | 'light';
  readonly gameLayerEnabled: boolean;
  readonly customColor: string | null;
}

type GameLayerEffect = PreferenceSideEffect<'gameLayerEnabled', boolean, TestPreferences>;

function definitions(gameLayerEffect?: GameLayerEffect): PreferenceRegistry<TestPreferences> {
  return {
    language: {
      key: 'language',
      defaultValue: 'system',
      normalize: (value) => (value === 'de' || value === 'en' ? value : 'system'),
      scope: 'synced',
    },
    theme: {
      key: 'theme',
      defaultValue: 'system',
      normalize: (value) =>
        value === 'dark' || value === 'light' || value === 'system' ? value : 'system',
      scope: 'identity',
    },
    gameLayerEnabled: {
      key: 'gameLayerEnabled',
      defaultValue: false,
      normalize: (value) => value === true,
      scope: 'synced',
      ...(gameLayerEffect ? { sideEffect: gameLayerEffect } : {}),
    },
    customColor: {
      key: 'customColor',
      defaultValue: null,
      normalize: (value) => (typeof value === 'string' ? value : null),
      scope: 'synced',
    },
  };
}

const defaults: TestPreferences = {
  language: 'system',
  theme: 'system',
  gameLayerEnabled: false,
  customColor: null,
};

function deferred<T>(): {
  readonly promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
} {
  let resolvePromise!: (value: T) => void;
  let rejectPromise!: (error: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  return { promise, resolve: resolvePromise, reject: rejectPromise };
}

function gatedStore(initialValues: TestPreferences): {
  readonly store: PreferenceStore<TestPreferences>;
  readonly writes: TestPreferences[];
  resolve(index: number): void;
  reject(index: number, error: Error): void;
} {
  let stored = initialValues;
  const writes: TestPreferences[] = [];
  const gates: ReturnType<typeof deferred<undefined>>[] = [];
  return {
    store: {
      read: () => Promise.resolve(stored),
      patch: (patch) => {
        const next = { ...stored, ...patch };
        const gate = deferred<undefined>();
        writes.push(next);
        gates.push(gate);
        return gate.promise.then(() => {
          stored = next;
          return stored;
        });
      },
    },
    writes,
    resolve: (index) => {
      gates[index]?.resolve(undefined);
    },
    reject: (index, error) => {
      gates[index]?.reject(error);
    },
  };
}

describe('preference controller hydration', () => {
  it('normalizes stored values and supplies defaults for omitted values', async () => {
    const store: PreferenceStore<TestPreferences> = {
      read: () =>
        Promise.resolve({ language: 'invalid', theme: 'dark' } as unknown as TestPreferences),
      patch: () => Promise.reject(new Error('unused')),
    };
    const visible = vi.fn();
    const controller = createPreferenceController({
      definitions: definitions(),
      store,
      onVisibleChange: visible,
    });

    await expect(controller.hydrate()).resolves.toEqual({ ...defaults, theme: 'dark' });
    expect(controller.getState()).toEqual({
      values: { ...defaults, theme: 'dark' },
      status: 'ready',
      error: null,
    });
  });
});

describe('preference controller updates', () => {
  it('shows an optimistic value and restores the previous value after a failed write', async () => {
    const failure = new Error('local write failed');
    const write = deferred<TestPreferences>();
    const store: PreferenceStore<TestPreferences> = {
      read: () => Promise.resolve(defaults),
      patch: () => write.promise,
    };
    const visible: TestPreferences[] = [];
    const controller = createPreferenceController({
      definitions: definitions(),
      store,
      onVisibleChange: (values) => visible.push(values),
    });
    await controller.hydrate();

    const update = controller.update({ theme: 'dark' });
    expect(controller.getState().values.theme).toBe('dark');
    write.reject(failure);

    await expect(update).rejects.toBe(failure);
    expect(controller.getState()).toEqual({ values: defaults, status: 'ready', error: failure });
    expect(visible.at(-1)).toEqual(defaults);
  });

  it('keeps optimistic updates as the default compatibility behavior', async () => {
    const firstWrite = deferred<TestPreferences>();
    const secondWrite = deferred<TestPreferences>();
    const patch = vi
      .fn<(value: Partial<TestPreferences>) => Promise<TestPreferences>>()
      .mockReturnValueOnce(firstWrite.promise)
      .mockReturnValueOnce(secondWrite.promise);
    const controller = createPreferenceController({
      definitions: definitions(),
      store: { read: () => Promise.resolve(defaults), patch },
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    const first = controller.update({ theme: 'dark' });
    const second = controller.update({ language: 'de' });

    expect(patch).toHaveBeenCalledTimes(2);
    expect(controller.getState().values).toEqual({ ...defaults, theme: 'dark', language: 'de' });
    expect('pendingCount' in controller.getState()).toBe(false);

    secondWrite.resolve({ ...defaults, language: 'de' });
    await expect(second).resolves.toEqual({ ...defaults, language: 'de' });
    firstWrite.resolve({ ...defaults, theme: 'dark' });
    await expect(first).resolves.toEqual({ ...defaults, theme: 'dark' });

    expect(controller.getState().values).toEqual({ ...defaults, theme: 'dark' });
  });

  it('does not patch storage when an old payload omits a preference', async () => {
    const stored = { ...defaults, language: 'de' as const };
    const patch = vi.fn<(value: Partial<TestPreferences>) => Promise<TestPreferences>>();
    patch.mockResolvedValue(stored);
    const controller = createPreferenceController({
      definitions: definitions(),
      store: { read: () => Promise.resolve(stored), patch },
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    await expect(controller.applyWireValue('language', { state: 'absent' })).resolves.toEqual(
      stored,
    );
    expect(patch).not.toHaveBeenCalled();
    expect(controller.getState().values.language).toBe('de');
  });

  it('applies explicit wire null without turning it into omission', async () => {
    const stored = { ...defaults, customColor: '#336699' };
    const store = new InMemoryPreferenceStore<TestPreferences>(defaults, stored);
    const controller = createPreferenceController({
      definitions: definitions(),
      store,
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    await controller.applyWireValue('customColor', { state: 'null' });
    await expect(store.read()).resolves.toEqual({ ...stored, customColor: null });
  });
});

describe('serialized preference controller updates', () => {
  it('serializes rapid writes against the latest committed values', async () => {
    const gated = gatedStore(defaults);
    const controller = createPreferenceController({
      definitions: definitions(),
      store: gated.store,
      onVisibleChange: () => undefined,
      updateMode: 'serialized',
    });
    await controller.hydrate();

    const theme = controller.update({ theme: 'dark' });
    const language = controller.update({ language: 'de' });

    expect(gated.writes).toEqual([{ ...defaults, theme: 'dark' }]);
    expect(controller.getState()).toMatchObject({ values: defaults, pendingCount: 2 });

    gated.resolve(0);
    await expect(theme).resolves.toEqual({ ...defaults, theme: 'dark' });
    await vi.waitFor(() => {
      expect(gated.writes).toHaveLength(2);
    });
    expect(gated.writes[1]).toEqual({ ...defaults, theme: 'dark', language: 'de' });
    expect(controller.getState()).toMatchObject({
      values: { ...defaults, theme: 'dark' },
      pendingCount: 1,
    });

    gated.resolve(1);
    await expect(language).resolves.toEqual({ ...defaults, theme: 'dark', language: 'de' });
    expect(controller.getState()).toEqual({
      values: { ...defaults, theme: 'dark', language: 'de' },
      status: 'ready',
      pendingCount: 0,
      error: null,
    });
  });

  it('continues after a failed first write without leaking its value', async () => {
    const gated = gatedStore(defaults);
    const controller = createPreferenceController({
      definitions: definitions(),
      store: gated.store,
      onVisibleChange: () => undefined,
      updateMode: 'serialized',
    });
    await controller.hydrate();

    const failed = controller.update({ theme: 'dark' });
    const next = controller.update({ language: 'de' });
    const failure = new Error('disk full');
    gated.reject(0, failure);

    await expect(failed).rejects.toBe(failure);
    await vi.waitFor(() => {
      expect(gated.writes).toHaveLength(2);
    });
    expect(gated.writes[1]).toEqual({ ...defaults, language: 'de' });
    gated.resolve(1);

    await expect(next).resolves.toEqual({ ...defaults, language: 'de' });
    expect(controller.getState()).toMatchObject({
      values: { ...defaults, language: 'de' },
      pendingCount: 0,
      error: null,
    });
  });

  it('rolls back a preview after a failed serialized write', async () => {
    const events: string[] = [];
    const failure = new Error('write failed');
    const write = deferred<TestPreferences>();
    const controller = createPreferenceController({
      definitions: definitions({
        mode: 'preview-with-rollback',
        onError: 'report',
        preview: () => events.push('preview'),
        rollback: () => events.push('rollback'),
        afterPersistence: () => events.push('persisted'),
      }),
      store: { read: () => Promise.resolve(defaults), patch: () => write.promise },
      onVisibleChange: () => undefined,
      updateMode: 'serialized',
    });
    await controller.hydrate();

    const update = controller.update({ gameLayerEnabled: true });
    await vi.waitFor(() => {
      expect(events).toEqual(['preview']);
    });
    expect(controller.getState()).toMatchObject({ values: defaults, pendingCount: 1 });
    write.reject(failure);

    await expect(update).rejects.toBe(failure);
    expect(events).toEqual(['preview', 'rollback']);
    expect(controller.getState()).toMatchObject({
      values: defaults,
      pendingCount: 0,
      error: failure,
    });
  });

  it('invalidates in-flight publication and queued writes after an identity switch', async () => {
    const old = gatedStore({ ...defaults, theme: 'dark' });
    const nextValues = { ...defaults, language: 'de' as const };
    const nextPatch = vi.fn<(patch: Partial<TestPreferences>) => Promise<TestPreferences>>();
    const visible: TestPreferences[] = [];
    const controller = createPreferenceController({
      definitions: definitions(),
      store: old.store,
      onVisibleChange: (values) => visible.push(values),
      updateMode: 'serialized',
    });
    await controller.hydrate();

    const inFlight = controller.update({ theme: 'light' });
    const queued = controller.update({ language: 'en' });
    await controller.switchIdentity({
      read: () => Promise.resolve(nextValues),
      patch: nextPatch,
    });

    old.resolve(0);
    await expect(inFlight).resolves.toEqual(nextValues);
    await expect(queued).resolves.toEqual(nextValues);

    expect(old.writes).toHaveLength(1);
    expect(nextPatch).not.toHaveBeenCalled();
    expect(visible.at(-1)).toEqual(nextValues);
    expect(controller.getState()).toEqual({
      values: nextValues,
      status: 'ready',
      pendingCount: 0,
      error: null,
    });
  });

  it('stops publishing pending and committed values during an in-flight write', async () => {
    const gated = gatedStore(defaults);
    const visible: TestPreferences[] = [];
    const controller = createPreferenceController({
      definitions: definitions(),
      store: gated.store,
      onVisibleChange: (values) => visible.push(values),
      updateMode: 'serialized',
    });
    await controller.hydrate();

    const update = controller.update({ theme: 'dark' });
    const publishedBeforeStop = visible.length;
    controller.stop();
    gated.resolve(0);

    await expect(update).resolves.toEqual({ ...defaults, theme: 'dark' });
    expect(visible).toHaveLength(publishedBeforeStop);
    expect(controller.getState()).toEqual({
      values: { ...defaults, theme: 'dark' },
      status: 'ready',
      pendingCount: 0,
      error: null,
    });
  });

  it('rejects unknown keys without adding them to the queue', async () => {
    const patch = vi.fn<(value: Partial<TestPreferences>) => Promise<TestPreferences>>();
    const controller = createPreferenceController({
      definitions: definitions(),
      store: { read: () => Promise.resolve(defaults), patch },
      onVisibleChange: () => undefined,
      updateMode: 'serialized',
    });
    await controller.hydrate();

    await expect(
      controller.update({ removedPreference: true } as unknown as Partial<TestPreferences>),
    ).rejects.toThrow('Unknown preference key: removedPreference');
    expect(patch).not.toHaveBeenCalled();
    expect(controller.getState()).toMatchObject({ pendingCount: 0, error: null });
  });

  it('reports pending-count transitions while committed values stay visible', async () => {
    const gated = gatedStore(defaults);
    const transitions: { pendingCount: number; theme: TestPreferences['theme'] }[] = [];
    const controllerRef: { current?: ReturnType<typeof createSerializedController> } = {};
    const controller = createSerializedController(gated.store, () => {
      const state = controllerRef.current?.getState();
      if (!state) {
        return;
      }
      transitions.push({ pendingCount: state.pendingCount, theme: state.values.theme });
    });
    controllerRef.current = controller;
    await controller.hydrate();

    const first = controller.update({ theme: 'dark' });
    const second = controller.update({ language: 'de' });
    expect(controller.getState().pendingCount).toBe(2);
    expect(controller.getState().values).toEqual(defaults);

    gated.resolve(0);
    await first;
    expect(controller.getState()).toMatchObject({
      values: { ...defaults, theme: 'dark' },
      pendingCount: 1,
    });
    await vi.waitFor(() => {
      expect(gated.writes).toHaveLength(2);
    });
    gated.resolve(1);
    await second;
    expect(controller.getState().pendingCount).toBe(0);
    expect(transitions).toContainEqual({ pendingCount: 2, theme: 'system' });
    expect(transitions.at(-1)).toEqual({ pendingCount: 0, theme: 'dark' });
  });
});

function createSerializedController(
  store: PreferenceStore<TestPreferences>,
  onVisibleChange: (values: TestPreferences) => void,
) {
  return createPreferenceController({
    definitions: definitions(),
    store,
    onVisibleChange,
    updateMode: 'serialized',
  });
}

describe('preference controller identity changes', () => {
  it('clears the previous identity before reading the next identity', async () => {
    const previous = { ...defaults, language: 'de' as const, theme: 'dark' as const };
    const next = { ...defaults, language: 'en' as const, theme: 'light' as const };
    const nextRead = deferred<TestPreferences | undefined>();
    const visible: TestPreferences[] = [];
    const controller = createPreferenceController({
      definitions: definitions(),
      store: new InMemoryPreferenceStore(defaults, previous),
      onVisibleChange: (values) => visible.push(values),
    });
    await controller.hydrate();

    const switched = controller.switchIdentity({
      read: () => nextRead.promise,
      patch: () => Promise.reject(new Error('unused')),
    });
    expect(controller.getState().values).toEqual(defaults);
    expect(visible.at(-1)).toEqual(defaults);
    nextRead.resolve(next);

    await expect(switched).resolves.toEqual(next);
    expect(visible.slice(-2)).toEqual([defaults, next]);
  });
});

describe('preference side effects', () => {
  it('runs the default side effect only after persistence succeeds', async () => {
    const write = deferred<TestPreferences>();
    const run = vi.fn();
    const controller = createPreferenceController({
      definitions: definitions({ mode: 'after-persistence', onError: 'report', run }),
      store: { read: () => Promise.resolve(defaults), patch: () => write.promise },
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    const update = controller.update({ gameLayerEnabled: true });
    expect(run).not.toHaveBeenCalled();
    write.resolve({ ...defaults, gameLayerEnabled: true });
    await update;

    expect(run).toHaveBeenCalledOnce();
    expect(run).toHaveBeenCalledWith(
      expect.objectContaining({ previousValue: false, value: true }),
    );
  });

  it('allows an explicit preview and rolls it back after persistence fails', async () => {
    const events: string[] = [];
    const failure = new Error('write failed');
    const controller = createPreferenceController({
      definitions: definitions({
        mode: 'preview-with-rollback',
        onError: 'report',
        preview: () => events.push('preview'),
        rollback: () => events.push('rollback'),
        afterPersistence: () => events.push('persisted'),
      }),
      store: {
        read: () => Promise.resolve(defaults),
        patch: () => Promise.reject(failure),
      },
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    await expect(controller.update({ gameLayerEnabled: true })).rejects.toBe(failure);
    expect(events).toEqual(['preview', 'rollback']);
    expect(controller.getState().values.gameLayerEnabled).toBe(false);
  });

  it('reports a post-persistence side-effect failure without undoing persisted state', async () => {
    const failure = new Error('query toggle failed');
    const store = new InMemoryPreferenceStore<TestPreferences>(defaults, defaults);
    const controller = createPreferenceController({
      definitions: definitions({
        mode: 'after-persistence',
        onError: 'report',
        run: () => Promise.reject(failure),
      }),
      store,
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    await expect(controller.update({ gameLayerEnabled: true })).rejects.toBe(failure);
    expect(controller.getState()).toEqual({
      values: { ...defaults, gameLayerEnabled: true },
      status: 'ready',
      error: failure,
    });
    await expect(store.read()).resolves.toEqual({ ...defaults, gameLayerEnabled: true });
  });
});

describe('preference controller visibility guard', () => {
  it('does not report a superseded identity side-effect failure on the new identity', async () => {
    const effect = deferred<undefined>();
    const write = deferred<TestPreferences>();
    const controller = createPreferenceController({
      definitions: definitions({
        mode: 'after-persistence',
        onError: 'report',
        run: () => effect.promise,
      }),
      store: { read: () => Promise.resolve(defaults), patch: () => write.promise },
      onVisibleChange: () => undefined,
    });
    await controller.hydrate();

    const update = controller.update({ gameLayerEnabled: true }).catch(() => undefined);
    write.resolve({ ...defaults, gameLayerEnabled: true });
    await Promise.resolve();
    await Promise.resolve();

    const next = { ...defaults, theme: 'light' as const };
    await controller.switchIdentity({
      read: () => Promise.resolve(next),
      patch: () => Promise.reject(new Error('unused')),
    });

    effect.reject(new Error('stale reminder rescheduling failed'));
    await update;

    expect(controller.getState()).toEqual({ values: next, status: 'ready', error: null });
  });

  it('stops publishing to the consumer after stop, while state still settles', async () => {
    const read = deferred<TestPreferences | undefined>();
    const visible: TestPreferences[] = [];
    const stored = { ...defaults, theme: 'dark' as const };
    const controller = createPreferenceController({
      definitions: definitions(),
      store: { read: () => read.promise, patch: () => Promise.reject(new Error('unused')) },
      onVisibleChange: (values) => visible.push(values),
    });

    const hydrated = controller.hydrate();
    controller.stop();
    read.resolve(stored);

    await expect(hydrated).resolves.toEqual(stored);
    expect(visible).toEqual([]);
    expect(controller.getState().values).toEqual(stored);
  });
});
