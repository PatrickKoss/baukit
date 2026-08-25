// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'web' as string };
const announceForAccessibility = vi.fn<(message: string) => void>();

vi.mock('react-native', () => ({
  get Platform() {
    return platform;
  },
  AccessibilityInfo: {
    announceForAccessibility: (message: string) => {
      announceForAccessibility(message);
    },
  },
}));

import { announce, DEFAULT_LIVE_REGION_ID } from './announce.js';

function region(id: string = DEFAULT_LIVE_REGION_ID): HTMLElement | null {
  return document.getElementById(id);
}

function liveRegion(id: string = DEFAULT_LIVE_REGION_ID): HTMLElement {
  const element = region(id);
  if (element === null) throw new Error(`no live region ${id}`);
  return element;
}

afterEach(() => {
  announceForAccessibility.mockReset();
  platform.OS = 'web';
  document.body.innerHTML = '';
});

describe('announce on native', () => {
  it.each(['ios', 'android'])('uses the platform accessibility API on %s', (os) => {
    platform.OS = os;

    announce('  Set saved  ', { assertive: true });

    expect(announceForAccessibility).toHaveBeenCalledWith('Set saved');
    expect(region()).toBeNull();
  });

  it('drops a blank message', () => {
    platform.OS = 'ios';

    announce('   ');

    expect(announceForAccessibility).not.toHaveBeenCalled();
  });
});

describe('announce on web', () => {
  it('creates one polite live region under the default id', () => {
    announce('Import complete');

    const live = region();
    expect(live).not.toBeNull();
    expect(live?.getAttribute('aria-live')).toBe('polite');
    expect(live?.getAttribute('role')).toBe('status');
    expect(live?.getAttribute('aria-atomic')).toBe('true');
    expect(live?.textContent).toBe('Import complete');
  });

  it('keeps the region out of sight', () => {
    announce('Saved');

    const style = region()?.style;
    expect(style?.position).toBe('absolute');
    expect(style?.width).toBe('1px');
    expect(style?.overflow).toBe('hidden');
  });

  it('raises urgency when assertive is requested', () => {
    announce('Sync failed', { assertive: true });

    expect(region()?.getAttribute('aria-live')).toBe('assertive');
    expect(region()?.getAttribute('role')).toBe('alert');
  });

  it('reuses the same region instead of stacking one per call', () => {
    announce('First');
    announce('Second');

    expect(document.querySelectorAll(`#${DEFAULT_LIVE_REGION_ID}`)).toHaveLength(1);
    expect(region()?.textContent).toBe('Second');
  });

  it('changes urgency on the region it already created', () => {
    announce('Saved');
    announce('Sync failed', { assertive: true });

    expect(document.querySelectorAll(`#${DEFAULT_LIVE_REGION_ID}`)).toHaveLength(1);
    expect(region()?.getAttribute('aria-live')).toBe('assertive');
  });

  it('places the region under a product-owned id', () => {
    announce('Saved', { liveRegionId: 'product-announcer' });

    expect(region('product-announcer')?.textContent).toBe('Saved');
    expect(region()).toBeNull();
  });

  it('adopts a region the product rendered itself', () => {
    const own = document.createElement('div');
    own.id = 'product-announcer';
    document.body.appendChild(own);

    announce('Saved', { liveRegionId: 'product-announcer' });

    expect(document.querySelectorAll('#product-announcer')).toHaveLength(1);
    expect(own.textContent).toBe('Saved');
  });

  it('clears the region so the same message twice is announced twice', () => {
    announce('Set saved');
    const live = liveRegion();

    const observer = new MutationObserver(() => undefined);
    observer.observe(live, { characterData: true, childList: true, subtree: true });

    announce('Set saved');
    const records = observer.takeRecords();
    observer.disconnect();

    // A screen reader re-reads the region only when its content actually
    // changes, so repeating a message has to clear it first.
    expect(records.length).toBeGreaterThan(1);
    expect(live.textContent).toBe('Set saved');
  });

  it('drops a blank message without disturbing the region', () => {
    announce('Set saved');

    announce('   ');

    expect(region()?.textContent).toBe('Set saved');
  });
});
