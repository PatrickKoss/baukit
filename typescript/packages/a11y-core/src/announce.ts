import { AccessibilityInfo, Platform } from 'react-native';

/** Default DOM id of the shared web live region. Override it per product. */
export const DEFAULT_LIVE_REGION_ID = 'baukit-announcer';

const VISUALLY_HIDDEN =
  'position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0;';

export interface AnnounceOptions {
  /** Interrupts the screen reader instead of waiting for a pause. */
  assertive?: boolean;
  /** DOM id of the web live region. Defaults to `DEFAULT_LIVE_REGION_ID`. */
  liveRegionId?: string;
}

function webLiveRegion(assertive: boolean, liveRegionId: string): HTMLElement | null {
  if (typeof document === 'undefined') return null;
  let region = document.getElementById(liveRegionId);
  if (!region) {
    region = document.createElement('div');
    region.id = liveRegionId;
    region.style.cssText = VISUALLY_HIDDEN;
    document.body.appendChild(region);
  }
  region.setAttribute('aria-atomic', 'true');
  region.setAttribute('aria-live', assertive ? 'assertive' : 'polite');
  region.setAttribute('role', assertive ? 'alert' : 'status');
  return region;
}

/** Announces an outcome without requiring a visible live-region component. */
export function announce(
  message: string,
  { assertive = false, liveRegionId = DEFAULT_LIVE_REGION_ID }: AnnounceOptions = {},
): void {
  const trimmed = message.trim();
  if (!trimmed) return;

  if (Platform.OS !== 'web') {
    AccessibilityInfo.announceForAccessibility(trimmed);
    return;
  }

  const region = webLiveRegion(assertive, liveRegionId);
  if (!region) return;
  region.textContent = '';
  // Force a distinct mutation so the same message twice is spoken twice.
  void region.offsetWidth;
  region.textContent = trimmed;
}
