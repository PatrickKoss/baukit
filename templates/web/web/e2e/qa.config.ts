/**
 * Product-owned input for the shared `qa-*` specs. Each spec iterates over this
 * file and nothing else, so a product changes its browser quality gate by
 * editing the lists below instead of editing specs.
 */

export interface QaApiStub {
  /** Glob passed to `page.route`, matched against the full request URL. */
  readonly url: string;
  readonly status: number;
  readonly body: unknown;
}

export interface QaRoute {
  /** Stable label used in test titles and axe attachment names. */
  readonly name: string;
  /** Path relative to the served app root. */
  readonly path: string;
  /** Accessible name of the level-1 heading the route must render. */
  readonly heading: string;
  /**
   * Accessible name of a control on this route that is inert when activated:
   * it must not navigate, open an overlay, or submit. The focus-ring check
   * clicks it and then tabs back to it, so a control with a side effect would
   * move focus somewhere the test can never return from.
   */
  readonly focusRingControl: string;
}

export interface QaOverlay {
  readonly name: string;
  /** Route to load before opening the overlay. */
  readonly path: string;
  /** Accessible name of the control that opens the overlay. */
  readonly trigger: string;
  /** Accessible name of the resulting `role="dialog"`. */
  readonly dialog: string;
  /** Accessible name of the control expected to hold focus on open. */
  readonly initialFocus: string;
  /** Accessible name of the control that closes it without committing. */
  readonly dismiss: string;
  /**
   * `data-testid` of the scrim, when an outside click is a supported dismissal.
   * Leave unset for an overlay that deliberately requires an explicit choice.
   */
  readonly scrimTestId?: string;
}

export interface QaSubmitTarget {
  readonly name: string;
  readonly path: string;
  /** Accessible name of the control that opens the form, when it is not inline. */
  readonly open?: string;
  /** Accessible name of a required field, and a value that satisfies it. */
  readonly field: string;
  readonly value: string;
  /** Accessible name of the submit control. */
  readonly submit: string;
  /** Text that must appear exactly once after any number of submit activations. */
  readonly result: string;
}

export interface QaRouteState {
  readonly name: string;
  /** Deep link that must land in a contextual state rather than a blank page. */
  readonly path: string;
  /** Heading the state renders. */
  readonly heading: string;
  /** Accessible name of the control that leaves the state. */
  readonly recovery: string;
}

export interface QaProtectedRoute {
  readonly name: string;
  readonly path: string;
  /** Glob for the API calls this route depends on. */
  readonly api: string;
  /** Text shown once those calls fail with 401. */
  readonly expiredMessage: string;
  /** Text that must not survive on screen once the session is gone. */
  readonly privateText: string;
}

export interface QaConfig {
  /** Walked by qa-axe, qa-keyboard, and qa-scroll. */
  readonly routes: readonly QaRoute[];
  /** Checked for focus trap, Escape, outside click, and focus restoration. */
  readonly overlays: readonly QaOverlay[];
  /** Checked for double-submit and post-submit history guards. */
  readonly submits: readonly QaSubmitTarget[];
  /** Deep links that must render a contextual route state. */
  readonly routeStates: readonly QaRouteState[];
  /** Routes whose private data must disappear when the session expires. */
  readonly protectedRoutes: readonly QaProtectedRoute[];
  /** CSS selector for the element each route's scroller must be flush with. */
  readonly screenSelector: string;
  /** Selectors and threshold used by qa-geometry. */
  readonly geometry: QaGeometry;
  /** Stubbed API responses, so the browser gate needs no backend. */
  readonly apiStubs: readonly QaApiStub[];
  /** Two accounts' worth of stub data, to prove one never leaks into the other. */
  readonly accounts: readonly QaAccount[];
}

export interface QaGeometry {
  readonly fixedNavigationSelector: string;
  readonly overlapTargetSelector: string;
  readonly scrollContainerSelector: string;
  readonly scrollTargetSelector: string;
  readonly interactiveTargetSelector: string;
  readonly minimumTargetSize: number;
}

export interface QaAccount {
  readonly name: string;
  /** Stubs applied while this account is signed in. */
  readonly apiStubs: readonly QaApiStub[];
  /** Text only this account may ever see. */
  readonly privateText: string;
}

const OWNER_ITEMS = [
  { id: '3f2a1b0c-4d5e-4f60-8a91-2b3c4d5e6f70', name: 'Owner item' },
  { id: '5c6d7e8f-9a0b-4c1d-8e2f-3a4b5c6d7e8f', name: 'Owner second item' },
];

const OTHER_ITEMS = [{ id: '9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a', name: 'Other item' }];

export const qaConfig: QaConfig = {
  routes: [
    {
      name: 'home',
      path: '/',
      heading: '{{ context.app_name }}',
      focusRingControl: 'Deny analytics',
    },
  ],
  overlays: [
    {
      name: 'accessible-dialog',
      path: '/',
      trigger: 'Open dialog example',
      dialog: 'Accessible dialog example',
      initialFocus: 'Example name',
      dismiss: 'Cancel',
    },
  ],
  submits: [
    {
      name: 'dialog-example',
      path: '/',
      open: 'Open dialog example',
      field: 'Example name',
      value: 'Guarded example',
      submit: 'Save example',
      result: 'Saved once.',
    },
  ],
  routeStates: [
    {
      name: 'invalid-deep-link',
      path: '/?item=not-a-uuid',
      heading: 'Invalid link',
      recovery: 'Back to items',
    },
    {
      name: 'missing-detail',
      path: '/?item=00000000-0000-4000-8000-000000000000',
      heading: 'Detail not found',
      recovery: 'Back to items',
    },
  ],
  protectedRoutes: [
    {
      name: 'items',
      path: '/',
      api: '**/items',
      expiredMessage: 'HTTP 401',
      privateText: 'Owner item',
    },
  ],
  accounts: [
    {
      name: 'owner',
      apiStubs: [{ url: '**/items', status: 200, body: OWNER_ITEMS }],
      privateText: 'Owner item',
    },
    {
      name: 'other',
      apiStubs: [{ url: '**/items', status: 200, body: OTHER_ITEMS }],
      privateText: 'Other item',
    },
  ],
  screenSelector: '.shell',
  geometry: {
    fixedNavigationSelector: '[data-testid="primary-navigation"]',
    overlapTargetSelector: '.consent .action:last-child',
    scrollContainerSelector: '.items',
    scrollTargetSelector: '.item:first-child',
    interactiveTargetSelector: 'a, button, input',
    minimumTargetSize: 44,
  },
  apiStubs: [{ url: '**/items', status: 200, body: OWNER_ITEMS }],
};
