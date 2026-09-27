import {
  createOwnedNotificationScheduler,
  decodeOwnedNotificationMarker,
  encodeOwnedNotificationMarker,
  NotificationPlanError,
  OWNED_NOTIFICATION_DATA_KEY,
  type NotificationPermission,
  type NotificationPlatform,
  type OwnedNotificationScheduler,
  type OwnedNotificationScheduleRequest,
  type OwnedNotificationSchedulerOptions,
  type PendingNotification,
} from '@baukit/notifications-core';
import type * as ExpoNotifications from 'expo-notifications';
import type {
  DateTriggerInput,
  NotificationContentInput,
  NotificationPermissionsStatus,
  NotificationRequest,
} from 'expo-notifications';

export const IOS_PENDING_NOTIFICATION_LIMIT = 64;

export type ExpoNotificationsApi = Pick<
  typeof ExpoNotifications,
  | 'getAllScheduledNotificationsAsync'
  | 'cancelScheduledNotificationAsync'
  | 'scheduleNotificationAsync'
  | 'getPermissionsAsync'
  | 'SchedulableTriggerInputTypes'
  | 'IosAuthorizationStatus'
>;

export interface ExpoNotificationContent {
  readonly content: NotificationContentInput;
  readonly channelId?: string;
}

export type ExpoOwnedNotificationSchedulerOptions = OwnedNotificationSchedulerOptions;

const UNDETERMINED = 'undetermined';

export function createExpoNotificationPlatform(
  api: ExpoNotificationsApi,
): NotificationPlatform<ExpoNotificationContent> {
  return {
    async list() {
      const requests = await api.getAllScheduledNotificationsAsync();
      return requests.map(pendingNotification);
    },
    cancel(identifier) {
      return api.cancelScheduledNotificationAsync(identifier);
    },
    async permission() {
      return permissionFrom(api, await api.getPermissionsAsync());
    },
    async schedule(request) {
      await api.scheduleNotificationAsync({
        identifier: request.identifier,
        content: contentWithMarker(request),
        trigger: dateTrigger(api, request),
      });
    },
  };
}

export function createExpoOwnedNotificationScheduler(
  api: ExpoNotificationsApi,
  options: ExpoOwnedNotificationSchedulerOptions = {},
): OwnedNotificationScheduler<ExpoNotificationContent> {
  const scheduler = createOwnedNotificationScheduler(createExpoNotificationPlatform(api), options);
  return {
    replaceOwned(owner, desired, replaceOptions) {
      const reserved = desired.find((entry) => hasReservedKey(entry.content.content));
      if (reserved !== undefined) {
        return Promise.reject(new NotificationPlanError('reserved_data_key', reserved.logicalId));
      }
      return scheduler.replaceOwned(owner, desired, replaceOptions);
    },
  };
}

function pendingNotification(request: NotificationRequest): PendingNotification {
  return {
    identifier: request.identifier,
    marker: decodeOwnedNotificationMarker(request.content.data?.[OWNED_NOTIFICATION_DATA_KEY]),
  };
}

function hasReservedKey(content: NotificationContentInput): boolean {
  return content.data !== undefined && Object.hasOwn(content.data, OWNED_NOTIFICATION_DATA_KEY);
}

function contentWithMarker(
  request: OwnedNotificationScheduleRequest<ExpoNotificationContent>,
): NotificationContentInput {
  const { content } = request.content;
  return {
    ...content,
    data: {
      ...content.data,
      [OWNED_NOTIFICATION_DATA_KEY]: encodeOwnedNotificationMarker(request.marker),
    },
  };
}

function dateTrigger(
  api: ExpoNotificationsApi,
  request: OwnedNotificationScheduleRequest<ExpoNotificationContent>,
): DateTriggerInput {
  const trigger: DateTriggerInput = {
    type: api.SchedulableTriggerInputTypes.DATE,
    date: request.marker.epochMilliseconds,
  };
  const { channelId } = request.content;
  return channelId === undefined ? trigger : { ...trigger, channelId };
}

function permissionFrom(
  api: ExpoNotificationsApi,
  permission: NotificationPermissionsStatus,
): NotificationPermission {
  const iosStatus = permission.ios?.status;
  if (
    permission.granted ||
    iosStatus === api.IosAuthorizationStatus.AUTHORIZED ||
    iosStatus === api.IosAuthorizationStatus.PROVISIONAL ||
    iosStatus === api.IosAuthorizationStatus.EPHEMERAL
  ) {
    return 'granted';
  }
  const status: string = permission.status;
  return status === UNDETERMINED ? 'undetermined' : 'denied';
}
