import { describe, expect, it } from 'vitest';

import { OperationQueue, queueForFile } from './operation-queue.js';

describe('OperationQueue', () => {
  it('runs operations one at a time in call order', async () => {
    const queue = new OperationQueue();
    const events: string[] = [];
    const operation = (name: string) => async (): Promise<string> => {
      events.push(`${name}:start`);
      await Promise.resolve();
      events.push(`${name}:end`);
      return name;
    };

    const results = await Promise.all([
      queue.run(operation('first')),
      queue.run(operation('second')),
    ]);

    expect(results).toEqual(['first', 'second']);
    expect(events).toEqual(['first:start', 'first:end', 'second:start', 'second:end']);
  });

  it('continues after a rejected operation', async () => {
    const queue = new OperationQueue();
    const failure = new Error('failed');
    const failed = queue.run(() => Promise.reject(failure));
    const next = queue.run(() => Promise.resolve('next'));

    await expect(failed).rejects.toBe(failure);
    await expect(next).resolves.toBe('next');
  });

  it('reports idle once every accepted operation settles', async () => {
    let idle = 0;
    const queue = new OperationQueue(() => {
      idle += 1;
    });

    await Promise.allSettled([
      queue.run(() => Promise.resolve()),
      queue.run(() => Promise.reject(new Error('failed'))),
    ]);

    expect(idle).toBe(1);
  });
});

describe('queueForFile', () => {
  it('shares a queue per file while work is pending and drops it when idle', async () => {
    let release = (): void => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const busy = queueForFile('/queue/shared.db');
    const running = busy.run(() => gate);

    expect(queueForFile('/queue/shared.db')).toBe(busy);
    expect(queueForFile('/queue/other.db')).not.toBe(busy);

    release();
    await running;
    await Promise.resolve();

    expect(queueForFile('/queue/shared.db')).not.toBe(busy);
  });
});
