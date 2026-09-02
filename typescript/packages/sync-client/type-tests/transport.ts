import {
  commitCursorAfterLocalTransaction,
  parseRetryAfter,
  SyncTransport,
  validatePullPage,
  validatePushOutcomeCoverage,
} from '@baukit/sync-client';

class ProductApiClient {
  request<T>(_path: string, _init: RequestInit = {}): Promise<T> {
    return Promise.reject(new Error('compile-only request'));
  }
}

const apiClient = new ProductApiClient();
const transport = new SyncTransport({ request: apiClient.request.bind(apiClient) });
const response: Promise<{ accepted: number }> = transport.request('/sync/push');
const retryAt: string = parseRetryAfter('5');
const page = validatePullPage(1, { nextCursor: 2, hasMore: false }, (left, right) => left - right);
const outcomes = validatePushOutcomeCoverage(
  [{ key: 'one' }],
  [{ key: 'one', result: 'accepted' }],
  { submittedKey: (item) => item.key, outcomeKey: (item) => item.key },
);
const committed: Promise<string> = commitCursorAfterLocalTransaction({
  nextCursor: 2,
  transaction: () => Promise.resolve('applied'),
  commitCursor: () => undefined,
});

export { committed, outcomes, page, response, retryAt, transport };
