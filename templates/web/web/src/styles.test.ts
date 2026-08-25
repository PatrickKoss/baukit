import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

const styles = readFileSync(fileURLToPath(new URL('./styles.css', import.meta.url)), 'utf8');

describe('interaction styling baseline', () => {
  it('provides visible keyboard focus and minimum effective touch targets', () => {
    expect(styles).toMatch(/:focus-visible\s*{/);
    expect(styles).toMatch(/min-block-size:\s*44px/);
    expect(styles).toMatch(/min-inline-size:\s*44px/);
  });
});
