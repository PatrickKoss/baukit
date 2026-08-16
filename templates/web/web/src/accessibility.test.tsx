// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { App } from './App';
import { AccessibleDialogExample } from './accessible-dialog';

vi.mock('./api', () => ({
  listItems: vi.fn().mockResolvedValue([]),
}));

afterEach(cleanup);

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

describe('generated accessibility scan', () => {
  it('has no serious or critical axe violations in the app shell', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const result = render(
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>,
    );

    await expectNoHighImpactViolations(result.container);
  });

  it('has no serious or critical axe violations in the open dialog', async () => {
    render(<AccessibleDialogExample />);
    fireEvent.click(
      screen.getByRole('button', { name: 'Open dialog example' }),
    );

    await expectNoHighImpactViolations(document.body);
  });
});
