import { it, expect, vi } from 'vitest';
import { createSuiteNavigationStore, createSuiteNoticeStore } from './navigation.js';

let nextId = 0;
it('consumes connection notices once', () => {
  const navigation = createSuiteNavigationStore(() => String(nextId++));
  const token = navigation.createSuiteConnectionNotice('request-1');
  expect(navigation.consumeSuiteNotice(token)).toEqual({
    notice: { connected: true, errorCode: null },
    announce: true,
  });
  expect(navigation.consumeSuiteNotice(token)).toBeNull();
  expect(navigation.createSuiteConnectionNotice('request-1')).toBe(token);
  expect(navigation.consumeSuiteNotice(token)).toBeNull();
});

it('keeps error codes in a one-shot notice rather than the URL', () => {
  const navigation = createSuiteNavigationStore(() => String(nextId++));
  const token = navigation.createSuiteErrorNotice('suite_peer_unreachable');
  expect(token).not.toBe('suite_peer_unreachable');
  expect(navigation.consumeSuiteNotice('suite_peer_unreachable')).toBeNull();
  expect(navigation.consumeSuiteNotice(token)).toEqual({
    notice: { connected: false, errorCode: 'suite_peer_unreachable' },
    announce: false,
  });
  expect(navigation.consumeSuiteNotice(token)).toBeNull();
});

it.each([undefined, null, 'connected', 'untrusted-token'])(
  'rejects an unissued notice %p',
  (token) => {
    expect(createSuiteNavigationStore(() => String(nextId++)).consumeSuiteNotice(token)).toBeNull();
  },
);

it('keeps provider stores independent', () => {
  const first = createSuiteNavigationStore(() => String(nextId++));
  const second = createSuiteNavigationStore(() => String(nextId++));
  const token = first.createSuiteConnectionNotice('request-1');
  expect(second.consumeSuiteNotice(token)).toBeNull();
  expect(first.claimSuiteConnectionAnnouncement('request-1')).toBe(true);
  expect(second.claimSuiteConnectionAnnouncement('request-1')).toBe(true);
  expect(first.claimSuiteConnectionAnnouncement('request-1')).toBe(false);
  expect(first.consumeSuiteNotice(token)?.announce).toBe(false);
});

it('does not repeat an announcement when the native start finishes first', () => {
  const navigation = createSuiteNavigationStore(() => String(nextId++));
  expect(navigation.claimSuiteConnectionAnnouncement('request-1')).toBe(true);
  const token = navigation.createSuiteConnectionNotice('request-1');
  expect(navigation.consumeSuiteNotice(token)).toEqual({
    notice: { connected: true, errorCode: null },
    announce: false,
  });
  expect(navigation.claimSuiteConnectionAnnouncement('request-1')).toBe(false);
  expect(navigation.claimSuiteConnectionAnnouncement('request-2')).toBe(true);
});

it('does not repeat an announcement when the linked route finishes first', () => {
  const navigation = createSuiteNavigationStore(() => String(nextId++));
  const token = navigation.createSuiteConnectionNotice('request-1');
  expect(navigation.consumeSuiteNotice(token)?.announce).toBe(true);
  expect(navigation.claimSuiteConnectionAnnouncement('request-1')).toBe(false);
  expect(navigation.createSuiteConnectionNotice('request-1')).toBe(token);
  expect(navigation.consumeSuiteNotice(token)).toBeNull();
});

it('replaces an obsolete pending notice with the latest completion', () => {
  const navigation = createSuiteNavigationStore(() => String(nextId++));
  const old = navigation.createSuiteConnectionNotice('request-1');
  const current = navigation.createSuiteErrorNotice('suite_peer_unreachable');
  expect(navigation.consumeSuiteNotice(old)).toBeNull();
  expect(navigation.consumeSuiteNotice(current)?.notice.errorCode).toBe('suite_peer_unreachable');
});

it('receives a notice idempotently when an effect runs twice', () => {
  const navigation = createSuiteNavigationStore(() => String(nextId++));
  const store = createSuiteNoticeStore(navigation);
  const listener = vi.fn();
  const unsubscribe = store.subscribe(listener);
  const params = {
    suiteResult: navigation.createSuiteConnectionNotice('request-1'),
    suiteError: undefined,
  };
  expect(store.receive(params)).toBe(true);
  expect(store.receive(params)).toBe(false);
  expect(store.getSnapshot()).toEqual({ connected: true, errorCode: null });
  expect(listener).toHaveBeenCalledTimes(1);
  store.clear();
  expect(store.getSnapshot()).toBeNull();
  expect(listener).toHaveBeenCalledTimes(2);
  unsubscribe();
  store.receive({
    suiteResult: undefined,
    suiteError: navigation.createSuiteErrorNotice('suite_peer_unreachable'),
  });
  expect(store.getSnapshot()).toEqual({ connected: false, errorCode: 'suite_peer_unreachable' });
  expect(listener).toHaveBeenCalledTimes(2);
});
