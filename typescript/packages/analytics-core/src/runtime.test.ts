import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  defaultUuidFactory,
  defaultWarning,
  getTimeoutRuntime,
  inferDevelopmentMode,
} from './runtime.js';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

describe('host runtime probes', () => {
  it('delegates timeouts to the global timer functions', () => {
    const setTimeout = vi.fn(() => 7);
    const clearTimeout = vi.fn();
    vi.stubGlobal('setTimeout', setTimeout);
    vi.stubGlobal('clearTimeout', clearTimeout);
    const callback = vi.fn();

    const runtime = getTimeoutRuntime();
    const handle = runtime.setTimeout(callback, 25);
    runtime.clearTimeout(handle);

    expect(setTimeout).toHaveBeenCalledWith(callback, 25);
    expect(clearTimeout).toHaveBeenCalledWith(7);
  });

  it('rejects runtimes without timers', () => {
    vi.stubGlobal('clearTimeout', undefined);
    expect(() => getTimeoutRuntime()).toThrow('requires setTimeout and clearTimeout');
  });

  it('infers development mode from NODE_ENV', () => {
    vi.stubEnv('NODE_ENV', 'production');
    expect(inferDevelopmentMode()).toBe(false);
    vi.stubEnv('NODE_ENV', 'test');
    expect(inferDevelopmentMode()).toBe(true);
    vi.stubGlobal('process', undefined);
    expect(inferDevelopmentMode()).toBe(true);
  });

  it('warns through console.warn when present and stays silent otherwise', () => {
    const warn = vi.fn();
    vi.stubGlobal('console', { warn });
    defaultWarning('careful');
    expect(warn).toHaveBeenCalledWith('careful');

    vi.stubGlobal('console', undefined);
    expect(() => {
      defaultWarning('ignored');
    }).not.toThrow();
  });

  it('returns crypto.randomUUID output and validates it', () => {
    vi.stubGlobal('crypto', { randomUUID: () => 'uuid-1' });
    expect(defaultUuidFactory()).toBe('uuid-1');

    vi.stubGlobal('crypto', { randomUUID: () => 1 });
    expect(() => defaultUuidFactory()).toThrow('return a string');

    vi.stubGlobal('crypto', {});
    expect(() => defaultUuidFactory()).toThrow('requires crypto.randomUUID');
  });
});
