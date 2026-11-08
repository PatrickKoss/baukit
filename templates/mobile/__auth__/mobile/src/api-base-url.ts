import Constants from 'expo-constants';

const configuredBaseUrl: unknown = Constants.expoConfig?.extra?.['apiBaseUrl'];

/** The API origin from Expo config; development falls back to the local API, production to the app's own origin. */
export const apiBaseUrl: string =
  typeof configuredBaseUrl === 'string'
    ? configuredBaseUrl
    : __DEV__
      ? 'http://localhost:{{ context.api_host_port }}'
      : '';
