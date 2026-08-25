// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';

import { hasDocument } from './platform.js';

describe('hasDocument', () => {
  it('is true in a document environment', () => {
    expect(hasDocument()).toBe(true);
  });

  it('is false where React Native runs, which has no document', () => {
    vi.stubGlobal('document', undefined);
    try {
      expect(hasDocument()).toBe(false);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
