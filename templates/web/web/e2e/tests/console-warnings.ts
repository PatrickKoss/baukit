import type { Page } from '@playwright/test';

export interface AllowedConsoleWarning {
  readonly message: string;
  readonly reason: string;
}

export const CONSOLE_WARNING_ALLOWLIST: readonly AllowedConsoleWarning[] = [
  {
    message: 'Service Worker registration blocked by Playwright',
    reason: 'The browser gate blocks service workers so page.route can observe every request.',
  },
];

export function isAllowedConsoleWarning(message: string): boolean {
  return CONSOLE_WARNING_ALLOWLIST.some((entry) => entry.message === message);
}

export function collectUnexpectedConsoleWarnings(page: Page): string[] {
  const unexpected: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'warning' && !isAllowedConsoleWarning(message.text())) {
      unexpected.push(message.text());
    }
  });
  return unexpected;
}
