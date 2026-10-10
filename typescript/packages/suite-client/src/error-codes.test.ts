import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import { SUITE_ERROR_CODES, type SuiteErrorCode } from './index.js';

it('matches the shared suite error code fixture', () => {
  const fixture: unknown = JSON.parse(
    readFileSync(
      new URL('../../../../fixtures/suite-events/v1/error-codes.json', import.meta.url),
      'utf8',
    ),
  );
  expect(fixture).toEqual({ schemaVersion: 1, codes: SUITE_ERROR_CODES });
  expect([...SUITE_ERROR_CODES].sort()).toEqual(SUITE_ERROR_CODES);
  expect(new Set(SUITE_ERROR_CODES).size).toBe(SUITE_ERROR_CODES.length);
  const code: SuiteErrorCode = 'suite_replay_window';
  expect(SUITE_ERROR_CODES).toContain(code);
});
