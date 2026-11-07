import { policyOrigins } from './origins';

describe('web export policy origins', () => {
  it('resolves origins from the public Expo env', () => {
    const origins = policyOrigins({
      EXPO_PUBLIC_API_URL: 'https://api.example.test/v1',
      EXPO_PUBLIC_POSTHOG_HOST: 'eu.posthog.example.test',
      EXPO_PUBLIC_MEDIA_ORIGIN: ' ',
    });

    expect(origins.api).toBe('https://api.example.test');
    expect(origins.analytics).toBe('https://eu.posthog.example.test');
    expect(origins.media).toBeUndefined();
  });
});
