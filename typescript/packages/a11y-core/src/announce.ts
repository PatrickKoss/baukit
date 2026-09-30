import { AccessibilityInfo, Platform } from 'react-native';

import { announce as announceOnWeb, type AnnounceOptions } from './announce-web.js';

export { DEFAULT_LIVE_REGION_ID, type AnnounceOptions } from './announce-web.js';

/** Announces an outcome without requiring a visible live-region component. */
export function announce(message: string, options: AnnounceOptions = {}): void {
  if (Platform.OS === 'web') {
    announceOnWeb(message, options);
    return;
  }
  const trimmed = message.trim();
  if (!trimmed) return;
  AccessibilityInfo.announceForAccessibility(trimmed);
}
