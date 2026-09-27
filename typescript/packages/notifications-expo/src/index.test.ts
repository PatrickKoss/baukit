import {
  decodeOwnedNotificationMarker,
  encodeOwnedNotificationMarker,
  NotificationPlatformFaultState,
  OWNED_NOTIFICATION_DATA_KEY,
  ownedNotificationIdentifier,
  type PendingNotification,
} from '@baukit/notifications-core';
import { describeOwnedNotificationSchedulerContract } from '@baukit/notifications-core/vitest';
import type {
  NotificationPermissionsStatus,
  NotificationRequest,
  NotificationRequestInput,
} from 'expo-notifications';
import { describe, expect, it, vi } from 'vitest';

import {
  createExpoNotificationPlatform,
  createExpoOwnedNotificationScheduler,
  type ExpoNotificationContent,
  type ExpoNotificationsApi,
  type ExpoOwnedNotificationSchedulerOptions,
} from './index.js';

const TRIGGER_TYPES = {
  CALENDAR: 'calendar',
  DAILY: 'daily',
  WEEKLY: 'weekly',
  MONTHLY: 'monthly',
  YEARLY: 'yearly',
  DATE: 'date',
  TIME_INTERVAL: 'timeInterval',
} as unknown as ExpoNotificationsApi['SchedulableTriggerInputTypes'];

const IOS_STATUS = {
  NOT_DETERMINED: 0,
  DENIED: 1,
  AUTHORIZED: 2,
  PROVISIONAL: 3,
  EPHEMERAL: 4,
} as unknown as ExpoNotificationsApi['IosAuthorizationStatus'];

const OWNER = { namespace: 'reminders' };
const INSTANT = Date.parse('2030-01-01T09:00:00Z');

function permissionStatus(
  permission: NotificationPlatformFaultState['permission'],
): NotificationPermissionsStatus {
  return {
    granted: permission === 'granted',
    status: permission,
    expires: 'never',
    canAskAgain: permission !== 'denied',
  } as NotificationPermissionsStatus;
}

function asResolved<T>(action: () => T): Promise<T> {
  return new Promise((resolve) => {
    resolve(action());
  });
}

class ExpoNotificationsMock {
  readonly faults = new NotificationPlatformFaultState();
  readonly requests = new Map<string, NotificationRequest>();
  readonly scheduled: NotificationRequestInput[] = [];
  readonly cancelAll = vi.fn(() => Promise.resolve());
  permissionOverride: NotificationPermissionsStatus | null = null;

  readonly api = {
    SchedulableTriggerInputTypes: TRIGGER_TYPES,
    IosAuthorizationStatus: IOS_STATUS,
    cancelAllScheduledNotificationsAsync: this.cancelAll,
    getAllScheduledNotificationsAsync: async (): Promise<NotificationRequest[]> => {
      await this.faults.beforeList();
      const snapshot = [...this.requests.values()];
      this.faults.afterList();
      return snapshot;
    },
    cancelScheduledNotificationAsync: (identifier: string): Promise<void> =>
      asResolved(() => {
        this.faults.beforeCancel(identifier);
        this.requests.delete(identifier);
      }),
    getPermissionsAsync: (): Promise<NotificationPermissionsStatus> =>
      asResolved(() => {
        this.faults.beforePermission();
        return this.permissionOverride ?? permissionStatus(this.faults.permission);
      }),
    scheduleNotificationAsync: (request: NotificationRequestInput): Promise<string> =>
      asResolved(() => {
        const identifier = request.identifier ?? 'generated';
        this.faults.beforeSchedule(identifier);
        this.scheduled.push(request);
        this.requests.set(identifier, {
          identifier,
          content: request.content,
          trigger: request.trigger,
        } as NotificationRequest);
        return identifier;
      }),
  };

  addForeign(identifier: string, data: Record<string, unknown> | null): void {
    this.requests.set(identifier, {
      identifier,
      content: { title: 'foreign', data },
      trigger: null,
    } as unknown as NotificationRequest);
  }

  pending(): PendingNotification[] {
    return [...this.requests.values()].map((request) => ({
      identifier: request.identifier,
      marker: decodeOwnedNotificationMarker(request.content.data?.[OWNED_NOTIFICATION_DATA_KEY]),
    }));
  }
}

function setup(options: ExpoOwnedNotificationSchedulerOptions = {}) {
  const mock = new ExpoNotificationsMock();
  const scheduler = createExpoOwnedNotificationScheduler(mock.api, options);
  return { mock, scheduler };
}

function desired(logicalId: string, content: ExpoNotificationContent) {
  return { logicalId, epochMilliseconds: INSTANT, contentDigest: 'v1', content };
}

describe('Expo owned notification scheduler', () => {
  describeOwnedNotificationSchedulerContract<ExpoNotificationContent>((options) => {
    const { mock, scheduler } = setup(options);
    return {
      scheduler,
      faults: mock.faults,
      content: (_logicalId, text) => ({ content: { title: text, body: text } }),
      addUnrelated: (identifier) => {
        mock.addForeign(identifier, { kind: 'other-feature' });
      },
      pending: () => mock.pending(),
    };
  });

  it('schedules a date trigger with the marker merged into product data', async () => {
    const { mock, scheduler } = setup();
    await scheduler.replaceOwned(OWNER, [
      desired('a', {
        content: { title: 'Time to train', data: { screen: 'plan' } },
        channelId: 'daily',
      }),
    ]);
    const identifier = ownedNotificationIdentifier(OWNER.namespace, 'a');
    expect(mock.scheduled).toEqual([
      {
        identifier,
        content: {
          title: 'Time to train',
          data: {
            screen: 'plan',
            [OWNED_NOTIFICATION_DATA_KEY]: encodeOwnedNotificationMarker({
              namespace: OWNER.namespace,
              logicalId: 'a',
              epochMilliseconds: INSTANT,
              contentDigest: 'v1',
            }),
          },
        },
        trigger: { type: 'date', date: INSTANT, channelId: 'daily' },
      },
    ]);
  });

  it('omits the channel when the product sets none', async () => {
    const { mock, scheduler } = setup();
    await scheduler.replaceOwned(OWNER, [desired('a', { content: { title: 'x' } })]);
    expect(mock.scheduled[0]?.trigger).toEqual({ type: 'date', date: INSTANT });
  });

  it('rejects product data that uses the reserved key before listing', async () => {
    const { mock, scheduler } = setup();
    mock.faults.failNextList();
    const content = { content: { title: 'x', data: { [OWNED_NOTIFICATION_DATA_KEY]: 'mine' } } };
    await expect(scheduler.replaceOwned(OWNER, [desired('a', content)])).rejects.toMatchObject({
      code: 'reserved_data_key',
      logicalId: 'a',
    });
    expect(await scheduler.replaceOwned(OWNER, [])).toMatchObject({
      failures: [{ code: 'list_failed' }],
    });
  });

  it('leaves requests with forged, invalid or missing markers alone', async () => {
    const { mock, scheduler } = setup();
    const marker = encodeOwnedNotificationMarker({
      namespace: OWNER.namespace,
      logicalId: 'a',
      epochMilliseconds: INSTANT,
      contentDigest: 'v1',
    });
    mock.addForeign('forged-identifier', { [OWNED_NOTIFICATION_DATA_KEY]: marker });
    mock.addForeign('invalid-marker', { [OWNED_NOTIFICATION_DATA_KEY]: '{"version":1}' });
    mock.addForeign('no-data', null);

    expect(await scheduler.replaceOwned(OWNER, [])).toMatchObject({
      status: 'complete',
      cancelled: [],
    });
    expect([...mock.requests.keys()].sort()).toEqual(
      ['forged-identifier', 'invalid-marker', 'no-data'].sort(),
    );
  });

  it('never cancels every scheduled notification', async () => {
    const { mock, scheduler } = setup();
    await scheduler.replaceOwned(OWNER, [desired('a', { content: { title: 'x' } })]);
    await scheduler.replaceOwned(OWNER, []);
    expect(mock.cancelAll).not.toHaveBeenCalled();
  });

  it.each([
    ['provisional', IOS_STATUS.PROVISIONAL],
    ['ephemeral', IOS_STATUS.EPHEMERAL],
    ['authorized', IOS_STATUS.AUTHORIZED],
  ])('treats iOS %s authorization as granted', async (_label, iosStatus) => {
    const { mock, scheduler } = setup();
    mock.permissionOverride = {
      ...permissionStatus('undetermined'),
      ios: { status: iosStatus },
    } as NotificationPermissionsStatus;
    expect(
      await scheduler.replaceOwned(OWNER, [desired('a', { content: { title: 'x' } })]),
    ).toMatchObject({ status: 'complete', scheduled: ['a'] });
  });

  it('treats an undetermined permission as not granted', async () => {
    const { mock, scheduler } = setup();
    mock.faults.setPermission('undetermined');
    expect(
      await scheduler.replaceOwned(OWNER, [desired('a', { content: { title: 'x' } })]),
    ).toMatchObject({
      status: 'incomplete',
      failures: [{ code: 'permission_denied', logicalId: 'a' }],
    });
  });

  it('reports the platform permission states', async () => {
    const mock = new ExpoNotificationsMock();
    const platform = createExpoNotificationPlatform(mock.api);
    mock.faults.setPermission('denied');
    expect(await platform.permission()).toBe('denied');
    mock.faults.setPermission('undetermined');
    expect(await platform.permission()).toBe('undetermined');
    mock.faults.setPermission('granted');
    expect(await platform.permission()).toBe('granted');
  });
});
