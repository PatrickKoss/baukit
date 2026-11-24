import Constants from 'expo-constants';

import { PRODUCT_NAME } from './product';

/** The deep-link scheme the build registered; a dev variant registers its own so it never shares redirects with the plain app. */
export function appScheme(): string {
  const configured: unknown = Constants.expoConfig?.scheme;
  const scheme: unknown = Array.isArray(configured) ? configured[0] : configured;
  return typeof scheme === 'string' && scheme !== '' ? scheme : PRODUCT_NAME;
}
