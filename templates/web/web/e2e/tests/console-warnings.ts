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

export interface CspViolation {
  readonly directive: string;
  readonly blockedUri: string;
  readonly sample: string;
}

const CSP_CONSOLE_MARKER = 'Content Security Policy';

/**
 * Collects every CSP violation the page reports, both as console errors and as
 * `securitypolicyviolation` events. There is no allowlist: the policy must fit the app.
 */
export async function collectContentSecurityPolicyViolations(page: Page): Promise<string[]> {
  const violations: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error' && message.text().includes(CSP_CONSOLE_MARKER)) {
      violations.push(message.text());
    }
  });
  await page.exposeFunction('reportCspViolation', (violation: CspViolation) => {
    violations.push(
      `securitypolicyviolation ${violation.directive} blocked ${violation.blockedUri} ${violation.sample}`.trim(),
    );
  });
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      const report = (window as unknown as { reportCspViolation: (violation: CspViolation) => void })
        .reportCspViolation;
      report({
        directive: event.effectiveDirective,
        blockedUri: event.blockedURI,
        sample: event.sample,
      });
    });
  });
  return violations;
}
