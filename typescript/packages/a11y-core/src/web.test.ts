// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';

// A plain React web app has no React Native to resolve. Importing it here is an
// error, so the web entry fails this file if anything it reaches still does.
vi.mock('react-native', () => {
  throw new Error('the web entry must not import react-native');
});

import * as web from './web.js';

describe('the web entry point', () => {
  it('loads without react-native and exports the DOM behavior', () => {
    expect(Object.keys(web).sort()).toEqual([
      'ARIA_HIDDEN_INERT_OPT_OUT',
      'activeFocusTarget',
      'applyFocusTrapKey',
      'asFocusContainer',
      'asFocusTarget',
      'asTreeElement',
      'createRouteFocusController',
      'focusOverlayEntry',
      'focusableElements',
      'hostElement',
      'makeOutsideSiblingsInert',
      'syncAriaHiddenInert',
      'useAriaHiddenInert',
      'useFocusTrap',
      'useInert',
      'useSingleFlight',
      'wrapFocusAtBoundary',
    ]);
  });
});
