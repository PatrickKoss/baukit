import { describe, expect, it, vi } from 'vitest';
import {
  SuiteClient,
  SuiteResponseError,
  suiteErrorCode,
  type AuthorizationInput,
  type SuiteDelivery,
  type SuiteLink,
  type SuiteTransport,
} from './index.js';

const link: SuiteLink = {
  id: 'link',
  peerApp: 'beta',
  role: 'initiator',
  remoteLinkId: 'remote',
  remoteDisplayName: 'User',
  status: 'active',
  sends: [],
  receives: [],
  shareXp: true,
  rewardMode: 'native',
  deliveryHealth: 'healthy',
  consecutiveFailures: 0,
  lastDeliveryAt: null,
  lastFailureAt: null,
  lastFailureCode: null,
  lastReceivedAt: null,
  replayEarliest: '2025-10-07',
  createdAt: '2026-10-07T00:00:00Z',
  updatedAt: '2026-10-07T00:00:00Z',
};
const input: AuthorizationInput = { client: 'alpha', state: 'state', codeChallenge: 'challenge' };
function apiFixture() {
  const request = vi.fn<(method: string, path: string, body?: unknown) => Promise<unknown>>(() =>
    Promise.resolve(link),
  );
  const transport: SuiteTransport = { request };
  return { client: new SuiteClient(transport), request };
}

describe('response validation', () => {
  const calls: readonly (readonly [string, (client: SuiteClient) => Promise<unknown>])[] = [
    ['peers', (c) => c.peers()],
    ['list', (c) => c.list()],
    ['get', (c) => c.get('id')],
    ['start', (c) => c.start({ peerApp: 'beta', returnUrl: 'r', stateNonce: 'n' })],
    ['complete', (c) => c.complete('request', 'code')],
    ['preview', (c) => c.preview('alpha')],
    ['authorize', (c) => c.authorize(input)],
    ['deny', (c) => c.deny({ client: 'alpha', state: 'state' })],
    ['update', (c) => c.update('id', { shareXp: false })],
    ['reenable', (c) => c.reenable('id')],
    ['deliveries', (c) => c.deliveries('id')],
  ];
  it.each(calls)('rejects a malformed %s response with SuiteResponseError', async (_name, call) => {
    const { client, request } = apiFixture();
    request.mockResolvedValueOnce({ unexpected: true });
    const error: unknown = await call(client).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(SuiteResponseError);
    expect(suiteErrorCode(error)).toBe('suite_unavailable');
  });
  it('rejects links and deliveries with invalid fields inside arrays', async () => {
    const { client, request } = apiFixture();
    request.mockResolvedValueOnce([{ ...link, status: 'bogus' }]);
    await expect(client.list()).rejects.toBeInstanceOf(SuiteResponseError);
    request.mockResolvedValueOnce([{ jobId: 'job' }]);
    await expect(client.deliveries('id')).rejects.toBeInstanceOf(SuiteResponseError);
    request.mockResolvedValueOnce([{ id: 'peer' }]);
    await expect(client.peers()).rejects.toBeInstanceOf(SuiteResponseError);
  });
  it('accepts a valid delivery list and ignores bodies of void endpoints', async () => {
    const { client, request } = apiFixture();
    const delivery: SuiteDelivery = {
      jobId: 'job',
      linkId: 'link',
      eventType: null,
      status: 'pending',
      attemptCount: 0,
      lastFailureCode: null,
      createdAt: '2026-10-07T00:00:00Z',
      updatedAt: '2026-10-07T00:00:00Z',
    };
    request.mockResolvedValueOnce([delivery]);
    expect(await client.deliveries('id')).toEqual([delivery]);
    request.mockResolvedValueOnce(undefined);
    await expect(client.disconnect('id')).resolves.toBeUndefined();
  });
});
