import { describe, expect, it } from 'vitest';

import { NotificationPlanError } from './errors.js';
import type { NotificationPlatformFaults } from './memory.js';
import { isOwnedBy, ownedNotificationIdentifier } from './ownership.js';
import type {
  NotificationOwner,
  OwnedNotification,
  OwnedNotificationReplacementOutcome,
  OwnedNotificationScheduler,
  OwnedNotificationSchedulerOptions,
  PendingNotification,
} from './scheduler.js';
import type { PlannedNotification } from './validation.js';

export interface OwnedNotificationSchedulerHarness<TContent> {
  readonly scheduler: OwnedNotificationScheduler<TContent>;
  readonly faults: NotificationPlatformFaults;
  content(logicalId: string, text: string): TContent;
  addUnrelated(identifier: string): Promise<void> | void;
  pending(): Promise<readonly PendingNotification[]> | readonly PendingNotification[];
}

export type OwnedNotificationSchedulerHarnessFactory<TContent> = (
  options: OwnedNotificationSchedulerOptions,
) =>
  | Promise<OwnedNotificationSchedulerHarness<TContent>>
  | OwnedNotificationSchedulerHarness<TContent>;

type Spec = readonly [logicalId: string, hour: number, contentDigest?: string];

const BASE_INSTANT = Date.parse('2030-01-01T00:00:00Z');
const MILLISECONDS_PER_HOUR = 3_600_000;
const PRIVATE_COPY = 'private copy';
const UNRELATED = 'unrelated-request';
const OWNER: NotificationOwner = { namespace: 'reminders' };
const PREFIX_SIBLING: NotificationOwner = { namespace: 'reminders-extra' };
const LIMIT = 4;

/** Registers the owned-notification replacement contract in the current Vitest suite. */
export function describeOwnedNotificationSchedulerContract<TContent>(
  makeHarness: OwnedNotificationSchedulerHarnessFactory<TContent>,
): void {
  const setup = async (options: OwnedNotificationSchedulerOptions = {}) => {
    const harness = await makeHarness(options);
    const build = (specs: readonly Spec[]): OwnedNotification<TContent>[] =>
      specs.map(([logicalId, hour, contentDigest = 'v1']) => ({
        logicalId,
        epochMilliseconds: BASE_INSTANT + hour * MILLISECONDS_PER_HOUR,
        contentDigest,
        content: harness.content(logicalId, `${PRIVATE_COPY} ${logicalId} ${contentDigest}`),
      }));
    const owned = async (owner: NotificationOwner = OWNER): Promise<PlannedNotification[]> =>
      (await harness.pending())
        .filter((request) => isOwnedBy(owner.namespace, request.identifier, request.marker))
        .map(({ marker }) => ({
          logicalId: marker?.logicalId ?? '',
          epochMilliseconds: marker?.epochMilliseconds ?? 0,
          contentDigest: marker?.contentDigest ?? '',
        }))
        .sort((left, right) => left.epochMilliseconds - right.epochMilliseconds);
    const identifiers = async (): Promise<string[]> =>
      (await harness.pending()).map(({ identifier }) => identifier).sort();
    const replace = (specs: readonly Spec[], owner = OWNER, replaceAll = false) =>
      harness.scheduler.replaceOwned(owner, build(specs), { replaceAll });
    const planned = (specs: readonly Spec[]): PlannedNotification[] =>
      build(specs).map(({ logicalId, epochMilliseconds, contentDigest }) => ({
        logicalId,
        epochMilliseconds,
        contentDigest,
      }));
    return { harness, owned, identifiers, replace, planned };
  };

  describe('OwnedNotificationScheduler contract', () => {
    it('schedules the desired set in instant order and reports completion', async () => {
      const { owned, replace, planned } = await setup();
      const outcome = await replace([
        ['b', 2],
        ['a', 1],
      ]);
      expect(outcome).toEqual(complete({ scheduled: ['a', 'b'] }));
      expect(await owned()).toEqual(
        planned([
          ['a', 1],
          ['b', 2],
        ]),
      );
    });

    it('keeps an unchanged set on repeated calls', async () => {
      const { owned, replace } = await setup();
      const specs: Spec[] = [
        ['a', 1],
        ['b', 2],
      ];
      await replace(specs);
      const before = await owned();
      expect(await replace(specs)).toEqual(complete({ kept: ['a', 'b'] }));
      expect(await replace([...specs].reverse())).toEqual(complete({ kept: ['a', 'b'] }));
      expect(await owned()).toEqual(before);
    });

    it('moves, removes, and adds by logical ID and instant', async () => {
      const { owned, replace, planned } = await setup();
      await replace([
        ['keep', 1],
        ['move', 2],
        ['drop', 3],
      ]);
      const outcome = await replace([
        ['keep', 1],
        ['move', 5],
        ['add', 4],
      ]);
      expect(outcome).toEqual(
        complete({ kept: ['keep'], cancelled: ['move', 'drop'], scheduled: ['add', 'move'] }),
      );
      expect(await owned()).toEqual(
        planned([
          ['keep', 1],
          ['add', 4],
          ['move', 5],
        ]),
      );
    });

    it('replaces content only when the digest changes or replaceAll is set', async () => {
      const { owned, replace, planned } = await setup();
      await replace([
        ['a', 1],
        ['b', 2],
      ]);
      expect(
        await replace([
          ['a', 1, 'v2'],
          ['b', 2],
        ]),
      ).toEqual(complete({ kept: ['b'], cancelled: ['a'], scheduled: ['a'] }));
      expect(
        await replace(
          [
            ['a', 1, 'v2'],
            ['b', 2],
          ],
          OWNER,
          true,
        ),
      ).toEqual(complete({ cancelled: ['a', 'b'], scheduled: ['a', 'b'] }));
      expect(await owned()).toEqual(
        planned([
          ['a', 1, 'v2'],
          ['b', 2],
        ]),
      );
    });

    it('leaves other namespaces and unrelated requests alone, including on disable', async () => {
      const { harness, owned, identifiers, replace, planned } = await setup();
      await harness.addUnrelated(UNRELATED);
      await replace([['shared-id', 1]], PREFIX_SIBLING);
      await replace([['shared-id', 2]]);

      expect(await replace([])).toEqual(complete({ cancelled: ['shared-id'] }));
      expect(await owned()).toEqual([]);
      expect(await owned(PREFIX_SIBLING)).toEqual(planned([['shared-id', 1]]));
      expect(await identifiers()).toEqual(
        [ownedNotificationIdentifier(PREFIX_SIBLING.namespace, 'shared-id'), UNRELATED].sort(),
      );
    });

    it('returns an incomplete outcome when listing fails and converges on retry', async () => {
      const { harness, owned, replace, planned } = await setup();
      harness.faults.failNextList();
      expect(await replace([['a', 1]])).toEqual(incomplete({}, [{ code: 'list_failed' }]));
      expect(await owned()).toEqual([]);
      expect(await replace([['a', 1]])).toEqual(complete({ scheduled: ['a'] }));
      expect(await owned()).toEqual(planned([['a', 1]]));
    });

    it('does not reschedule an item whose cancel failed and converges on retry', async () => {
      const { harness, owned, replace, planned } = await setup();
      await replace([
        ['a', 1],
        ['b', 2],
      ]);
      harness.faults.failCancel(ownedNotificationIdentifier(OWNER.namespace, 'a'));
      const specs: Spec[] = [
        ['a', 3],
        ['b', 4],
      ];
      expect(await replace(specs)).toEqual(
        incomplete({ cancelled: ['b'], scheduled: ['b'] }, [
          { code: 'cancel_failed', logicalId: 'a' },
        ]),
      );
      expect(await owned()).toEqual(
        planned([
          ['a', 1],
          ['b', 4],
        ]),
      );
      expect(await replace(specs)).toEqual(
        complete({ kept: ['b'], cancelled: ['a'], scheduled: ['a'] }),
      );
      expect(await owned()).toEqual(planned(specs));
    });

    it('reports one failed schedule and converges on retry', async () => {
      const { harness, owned, replace, planned } = await setup();
      harness.faults.failSchedule(ownedNotificationIdentifier(OWNER.namespace, 'b'));
      const specs: Spec[] = [
        ['a', 1],
        ['b', 2],
        ['c', 3],
      ];
      expect(await replace(specs)).toEqual(
        incomplete({ scheduled: ['a', 'c'] }, [{ code: 'schedule_failed', logicalId: 'b' }]),
      );
      expect(await replace(specs)).toEqual(complete({ kept: ['a', 'c'], scheduled: ['b'] }));
      expect(await owned()).toEqual(planned(specs));
    });

    it('cancels stale requests but schedules nothing without permission', async () => {
      const { harness, owned, replace, planned } = await setup();
      await replace([
        ['a', 1],
        ['b', 2],
      ]);
      harness.faults.setPermission('denied');
      expect(
        await replace([
          ['a', 1],
          ['c', 3],
        ]),
      ).toEqual(
        incomplete({ kept: ['a'], cancelled: ['b'] }, [
          { code: 'permission_denied', logicalId: 'c' },
        ]),
      );
      expect(await owned()).toEqual(planned([['a', 1]]));
      expect(await replace([])).toEqual(complete({ cancelled: ['a'] }));
    });

    it('notices permission revoked after listing', async () => {
      const { harness, replace } = await setup();
      harness.faults.afterNextList(() => {
        harness.faults.setPermission('denied');
      });
      expect(await replace([['a', 1]])).toEqual(
        incomplete({}, [{ code: 'permission_denied', logicalId: 'a' }]),
      );
    });

    it('reports a failed permission check', async () => {
      const { harness, replace } = await setup();
      harness.faults.failNextPermission();
      expect(await replace([['a', 1]])).toEqual(
        incomplete({}, [{ code: 'permission_failed', logicalId: 'a' }]),
      );
    });

    it('stops at the pending limit, counting other owners, and keeps the earliest', async () => {
      const { harness, owned, replace, planned } = await setup({ pendingLimit: LIMIT });
      await harness.addUnrelated(UNRELATED);
      await replace([['other', 1]], PREFIX_SIBLING);
      expect(
        await replace([
          ['c', 3],
          ['a', 1],
          ['b', 2],
        ]),
      ).toEqual(
        incomplete({ scheduled: ['a', 'b'] }, [{ code: 'schedule_limit', logicalId: 'c' }]),
      );
      expect(await owned()).toEqual(
        planned([
          ['a', 1],
          ['b', 2],
        ]),
      );
    });

    it('coalesces queued replacements for one owner to the newest set', async () => {
      const { harness, owned, replace, planned } = await setup();
      const release = harness.faults.holdNextList();
      const first = replace([['first', 1]]);
      const second = replace([['second', 2]]);
      const third = replace([['third', 3]]);
      release();

      expect(await second).toEqual({
        status: 'superseded',
        kept: [],
        cancelled: [],
        scheduled: [],
        failures: [],
      });
      expect(await first).toEqual(complete({ scheduled: ['first'] }));
      expect(await third).toEqual(complete({ cancelled: ['first'], scheduled: ['third'] }));
      expect(await owned()).toEqual(planned([['third', 3]]));
    });

    it('does not make different owners wait for each other', async () => {
      const { harness, owned, replace, planned } = await setup();
      const release = harness.faults.holdNextList();
      const held = replace([['a', 1]]);
      expect(await replace([['b', 2]], PREFIX_SIBLING)).toEqual(complete({ scheduled: ['b'] }));
      release();
      expect(await held).toEqual(complete({ scheduled: ['a'] }));
      expect(await owned()).toEqual(planned([['a', 1]]));
      expect(await owned(PREFIX_SIBLING)).toEqual(planned([['b', 2]]));
    });

    it('rejects invalid owners and duplicate logical IDs before touching the platform', async () => {
      const { harness, identifiers, replace } = await setup();
      harness.faults.failNextList();
      await expect(replace([['a', 1]], { namespace: 'Bad Namespace' })).rejects.toMatchObject({
        code: 'invalid_namespace',
      });
      await expect(
        replace([
          ['a', 1],
          ['a', 2],
        ]),
      ).rejects.toBeInstanceOf(NotificationPlanError);
      expect(await replace([])).toEqual(incomplete({}, [{ code: 'list_failed' }]));
      expect(await identifiers()).toEqual([]);
    });

    it('keeps notification content out of outcomes', async () => {
      const { harness, replace } = await setup({ pendingLimit: 1 });
      harness.faults.failSchedule(ownedNotificationIdentifier(OWNER.namespace, 'a'));
      const outcome = await replace([
        ['a', 1],
        ['b', 2],
        ['c', 3],
      ]);
      expect(outcome.status).toBe('incomplete');
      expect(JSON.stringify(outcome)).not.toContain(PRIVATE_COPY);
    });
  });
}

type OutcomeLists = Partial<
  Pick<OwnedNotificationReplacementOutcome, 'kept' | 'cancelled' | 'scheduled'>
>;

function complete(lists: OutcomeLists): OwnedNotificationReplacementOutcome {
  return { status: 'complete', ...emptyLists(lists), failures: [] };
}

function incomplete(
  lists: OutcomeLists,
  failures: OwnedNotificationReplacementOutcome['failures'],
): OwnedNotificationReplacementOutcome {
  return { status: 'incomplete', ...emptyLists(lists), failures };
}

function emptyLists(lists: OutcomeLists) {
  return {
    kept: lists.kept ?? [],
    cancelled: lists.cancelled ?? [],
    scheduled: lists.scheduled ?? [],
  };
}
