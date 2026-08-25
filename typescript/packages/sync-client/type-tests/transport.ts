import { SyncTransport } from '@baukit/sync-client';

class ProductApiClient {
  request<T>(_path: string, _init: RequestInit = {}): Promise<T> {
    return Promise.reject(new Error('compile-only request'));
  }
}

const apiClient = new ProductApiClient();
const transport = new SyncTransport({ request: apiClient.request.bind(apiClient) });
const response: Promise<{ accepted: number }> = transport.request('/sync/push');

export { response, transport };
