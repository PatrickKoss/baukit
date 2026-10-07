export interface SuiteNavigationNotice {
  errorCode: string | null;
  connected: boolean;
}

interface PendingNotice {
  token: string;
  notice: SuiteNavigationNotice;
  announced: boolean;
}

export type SuiteNavigationStore = ReturnType<typeof createSuiteNavigationStore>;

export function createSuiteNavigationStore(randomUUID: () => string) {
  const connectionNotices = new Map<string, PendingNotice>();
  const pendingNotices = new Map<string, PendingNotice>();

  function createNotice(notice: SuiteNavigationNotice): PendingNotice {
    const pending = { token: randomUUID(), notice, announced: false };
    pendingNotices.clear();
    pendingNotices.set(pending.token, pending);
    return pending;
  }

  function connectionNotice(requestId: string): PendingNotice {
    const existing = connectionNotices.get(requestId);
    if (existing) return existing;
    const pending = createNotice({ connected: true, errorCode: null });
    connectionNotices.set(requestId, pending);
    return pending;
  }

  function createSuiteConnectionNotice(requestId: string): string {
    return connectionNotice(requestId).token;
  }

  function claimSuiteConnectionAnnouncement(requestId: string): boolean {
    const pending = connectionNotice(requestId);
    if (pending.announced) return false;
    pending.announced = true;
    return true;
  }

  function createSuiteErrorNotice(code: string): string {
    return createNotice({ connected: false, errorCode: code }).token;
  }

  function consumeSuiteNotice(token: unknown) {
    if (typeof token !== 'string') return null;
    const pending = pendingNotices.get(token);
    if (!pending) return null;
    pendingNotices.delete(token);
    const announce = pending.notice.connected && !pending.announced;
    pending.announced = true;
    return { notice: pending.notice, announce };
  }

  return {
    createSuiteConnectionNotice,
    claimSuiteConnectionAnnouncement,
    createSuiteErrorNotice,
    consumeSuiteNotice,
  };
}

export function createSuiteNoticeStore(navigation: SuiteNavigationStore) {
  let notice: SuiteNavigationNotice | null = null;
  let receivedToken: string | undefined;
  const listeners = new Set<() => void>();
  function publish(next: SuiteNavigationNotice | null) {
    notice = next;
    for (const listener of listeners) listener();
  }
  return {
    getSnapshot: () => notice,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    receive(params: { suiteResult: string | undefined; suiteError: string | undefined }): boolean {
      const token = params.suiteError ?? params.suiteResult;
      if (token === receivedToken) return false;
      receivedToken = token;
      const received = navigation.consumeSuiteNotice(token);
      publish(received?.notice ?? null);
      return received?.announce ?? false;
    },
    clear() {
      publish(null);
    },
  };
}
