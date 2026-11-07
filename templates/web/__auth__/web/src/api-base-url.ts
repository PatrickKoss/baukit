const configuredBaseUrl: unknown = import.meta.env.VITE_API_URL;

/** The backend origin that serves the API and `/auth/session/*`. */
export const apiBaseUrl: string =
  typeof configuredBaseUrl === 'string' && configuredBaseUrl !== ''
    ? configuredBaseUrl
    : import.meta.env.MODE === 'production'
      ? window.location.origin
      : 'http://localhost:{{ context.api_host_port }}';
