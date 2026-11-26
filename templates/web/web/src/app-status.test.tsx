// @vitest-environment jsdom

import { act, cleanup, render, screen } from '@testing-library/react';
import axe from 'axe-core';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createApiAvailabilityStore } from './api-availability';
import { unavailableMessage } from './api-status';
import { ApiUnavailableBanner, AppErrorBoundary } from './app-status';
import { germanCatalog } from './localization/de';
import { englishCatalog } from './localization/en';
import { createAppQueryClient } from './query-client';

afterEach(cleanup);

const RETRY_AT = Date.UTC(2030, 0, 1, 14, 5);

async function expectNoHighImpactViolations(root: Element): Promise<void> {
  const results = await axe.run(root, {
    rules: {
      // jsdom has no canvas/layout engine, so real-browser review owns contrast.
      'color-contrast': { enabled: false },
    },
  });
  const violations = results.violations.filter(
    ({ impact }) => impact === 'serious' || impact === 'critical',
  );
  expect(violations).toEqual([]);
}

function Broken(): ReactElement {
  throw new Error('render failed');
}

describe('api unavailable banner', () => {
  it('announces the cooldown in a polite live region and clears it', async () => {
    const store = createApiAvailabilityStore();
    const result = render(<ApiUnavailableBanner store={store} />);
    const region = screen.getByRole('status');
    expect(region.textContent).toBe('');

    act(() => {
      store.markUnavailable(RETRY_AT);
    });
    expect(region.textContent).toBe(unavailableMessage(RETRY_AT));
    expect(region.textContent).toMatch(/^The service is unavailable\. Retrying at .+\.$/u);
    await expectNoHighImpactViolations(result.container);

    act(() => {
      store.markAvailable();
    });
    expect(region.textContent).toBe('');
  });

  it('formats the retry time for German', () => {
    const time = new Intl.DateTimeFormat('de-DE', { hour: '2-digit', minute: '2-digit' }).format(RETRY_AT);
    expect(unavailableMessage(RETRY_AT, 'de-DE')).toBe(
      `Der Dienst ist nicht erreichbar. Neuer Versuch um ${time}.`,
    );
  });
});

describe('app error boundary', () => {
  it('replaces a crashed tree with an accessible alert', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const result = render(
      <AppErrorBoundary>
        <Broken />
      </AppErrorBoundary>,
    );

    expect(screen.getByRole('alert').textContent).toContain('Something went wrong');
    expect(screen.getByRole('button', { name: 'Reload' })).toBeDefined();
    await expectNoHighImpactViolations(result.container);
    vi.restoreAllMocks();
  });
});

describe('status catalog', () => {
  it('has the same keys in German as in English', () => {
    expect(Object.keys(germanCatalog.status).sort()).toEqual(Object.keys(englishCatalog.status).sort());
  });
});

describe('query client', () => {
  it('leaves retries to the api runtime and does not refetch on focus', () => {
    const defaults = createAppQueryClient().getDefaultOptions();
    expect(defaults.queries?.retry).toBe(false);
    expect(defaults.queries?.refetchOnWindowFocus).toBe(false);
    expect(defaults.mutations?.retry).toBe(false);
  });
});
