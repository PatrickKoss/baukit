/**
 * Response headers every page of a built web app carries next to its policy.
 * COEP stays unset: `require-corp` blocks third-party images and analytics
 * that do not send CORP themselves.
 */
export const SECURITY_HEADERS: Readonly<Record<string, string>> = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Resource-Policy': 'same-origin',
  'X-Content-Type-Options': 'nosniff',
};
