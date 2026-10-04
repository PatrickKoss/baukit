import { act, render, renderHook, waitFor } from '@testing-library/react-native';
import { PersistenceIdentityMismatchError } from '@baukit/data-contracts';

const mockSecureStore = new Map<string, string>();
function mockValidateSecureStoreKey(key: string): void {
  if (!/^[\w.-]+$/.test(key)) throw new Error('Invalid SecureStore key');
}
const mockAnalyticsReset = jest.fn();
const mockOpenDatabase = jest.fn((name: string) => Promise.resolve({ name }));
const mockStoreInitialize = jest.fn(() => Promise.resolve());
const mockStoreClose = jest.fn(() => Promise.resolve());
const mockStoreConstructed = jest.fn();
const mockDeleteDatabase = jest.fn<Promise<void>, [string]>().mockResolvedValue(undefined);

jest.mock('expo-secure-store', () => ({
  getItemAsync: (key: string) => {
    mockValidateSecureStoreKey(key);
    return Promise.resolve(mockSecureStore.get(key) ?? null);
  },
  setItemAsync: (key: string, value: string) => {
    mockValidateSecureStoreKey(key);
    mockSecureStore.set(key, value);
    return Promise.resolve();
  },
}));
jest.mock('expo-crypto', () => {
  const { createHash } = jest.requireActual<typeof import('node:crypto')>('node:crypto');
  return {
    CryptoDigestAlgorithm: { SHA256: 'SHA-256' },
    digestStringAsync: (_algorithm: string, value: string) =>
      Promise.resolve(createHash('sha256').update(value).digest('hex')),
  };
});
jest.mock('expo-sqlite', () => ({
  openDatabaseAsync: (name: string) => mockOpenDatabase(name),
  deleteDatabaseAsync: (name: string) => mockDeleteDatabase(name),
}));
jest.mock('@baukit/data-contracts-expo-sqlite', () => ({
  ExpoSqliteStore: class {
    public constructor(...parameters: unknown[]) {
      mockStoreConstructed(...parameters);
    }

    public initialize() {
      return mockStoreInitialize();
    }

    public close() {
      return mockStoreClose();
    }
  },
}));
jest.mock('./analytics', () => ({
  loadAnalytics: () => Promise.resolve({ reset: mockAnalyticsReset }),
}));

import {
  type AuthenticatedLocalData,
  AuthenticatedLocalDataProvider,
  useAuthenticatedLocalData,
} from './local-data';

interface Session {
  readonly subject: string | undefined;
  readonly sessionExpired: boolean;
}

interface LocalDataProbeProps {
  readonly onRender: (localData: AuthenticatedLocalData) => void;
}

function LocalDataProbe({ onRender }: LocalDataProbeProps) {
  onRender(useAuthenticatedLocalData());
  return null;
}

async function renderLocalData(initial: Session) {
  let latest: AuthenticatedLocalData | undefined;
  const record = (localData: AuthenticatedLocalData) => {
    latest = localData;
  };
  const tree = (session: Session) => (
    <AuthenticatedLocalDataProvider {...session}>
      <LocalDataProbe onRender={record} />
    </AuthenticatedLocalDataProvider>
  );
  const rendered = await render(tree(initial));
  return {
    result: {
      get current(): AuthenticatedLocalData {
        if (latest === undefined) throw new Error('The probe has not rendered.');
        return latest;
      },
    },
    rerender: (session: Session) => rendered.rerender(tree(session)),
    unmount: rendered.unmount,
  };
}

async function renderReady(subject: string) {
  const rendered = await renderLocalData({ subject, sessionExpired: false });
  await waitFor(() => {
    expect(rendered.result.current.state.status).toBe('ready');
  });
  mockStoreClose.mockClear();
  return rendered;
}

describe('authenticated local data', () => {
  it('requires the provider', async () => {
    jest.spyOn(console, 'error').mockImplementation(() => undefined);

    await expect(renderHook(() => useAuthenticatedLocalData())).rejects.toThrow(
      'useAuthenticatedLocalData must be used within AuthenticatedLocalDataProvider.',
    );
  });

  it('opens the subject partition in its own database', async () => {
    const { result } = await renderReady('subject-open');

    const state = result.current.state;
    if (state.status !== 'ready') throw new Error('Expected a ready partition.');
    expect(state.partition.subject).toBe('subject-open');
    expect(mockOpenDatabase).toHaveBeenCalledWith(`${state.partition.storeName}.db`);
    expect(mockStoreConstructed).toHaveBeenCalledWith(
      { name: `${state.partition.storeName}.db` },
      'product',
      { closeDatabase: true },
    );
    expect(mockAnalyticsReset).toHaveBeenCalled();
    const registry = [...mockSecureStore.entries()].find(([key]) =>
      key.endsWith('.local-data-registry.v1'),
    );
    expect(registry).toBeDefined();
    expect(registry?.[1]).toContain('subject-open');
  });

  it('closes the partition on sign-out', async () => {
    const { result, rerender } = await renderReady('subject-sign-out');

    await rerender({ subject: undefined, sessionExpired: false });

    await waitFor(() => {
      expect(result.current.state.status).toBe('signed-out');
    });
    expect(mockStoreClose).toHaveBeenCalledTimes(1);
  });

  it('deletes the closed partition database and removes its registry entry', async () => {
    const { result } = await renderReady('subject-erase');
    const state = result.current.state;
    if (state.status !== 'ready') throw new Error('Expected a ready partition.');
    await act(() => result.current.erase('subject-erase'));
    expect(mockStoreClose).toHaveBeenCalledTimes(1);
    expect(mockDeleteDatabase).toHaveBeenCalledWith(`${state.partition.storeName}.db`);
    expect(result.current.state.status).toBe('signed-out');
    const registry = [...mockSecureStore.entries()].find(([key]) => key.endsWith('.local-data-registry.v1'));
    expect(registry?.[1]).not.toContain('subject-erase');
  });

  it('blocks local data when the session expires', async () => {
    const { result, rerender } = await renderReady('subject-expired');

    await rerender({ subject: 'subject-expired', sessionExpired: true });

    await waitFor(() => {
      expect(result.current.state).toMatchObject({ status: 'blocked', reason: 'session-expired' });
    });
    expect(mockStoreClose).toHaveBeenCalledTimes(1);
  });

  it('blocks local data after an identity mismatch', async () => {
    const { result } = await renderReady('subject-mismatch');
    const error = new PersistenceIdentityMismatchError();

    await act(() => result.current.blockIdentityMismatch(error));

    expect(result.current.state).toEqual({ status: 'blocked', reason: 'identity-mismatch', error });
    expect(mockStoreClose).toHaveBeenCalledTimes(1);
  });

  it('closes a store that fails to initialize and reports the failure', async () => {
    const failure = new Error('disk full');
    mockStoreInitialize.mockRejectedValueOnce(failure);
    mockStoreClose.mockRejectedValueOnce(new Error('already closed'));

    const { result } = await renderLocalData({ subject: 'subject-failed', sessionExpired: false });

    await waitFor(() => {
      expect(result.current.state).toEqual({
        status: 'blocked',
        reason: 'initialization-failed',
        error: failure,
      });
    });
    expect(mockStoreClose).toHaveBeenCalledTimes(1);
  });

  it('ignores a transition that settles after unmount', async () => {
    let finishInitialize: () => void = () => undefined;
    mockStoreInitialize.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        finishInitialize = resolve;
      }),
    );
    const { result, unmount } = await renderLocalData({
      subject: 'subject-unmounted',
      sessionExpired: false,
    });
    expect(result.current.state.status).toBe('initializing');

    await unmount();
    await act(() => {
      finishInitialize();
    });

    expect(mockStoreInitialize).toHaveBeenCalledTimes(1);
  });
});


it('retains a different account database if the subject changes during erasure', async () => {
  const { result } = await renderReady('subject-new-account');
  await expect(result.current.erase('subject-old-account')).rejects.toThrow(PersistenceIdentityMismatchError);
  expect(mockDeleteDatabase).not.toHaveBeenCalled();
  expect(result.current.state).toMatchObject({ status: 'ready', partition: { subject: 'subject-new-account' } });
});
