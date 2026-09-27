import AsyncStorage from '@react-native-async-storage/async-storage';
import * as Crypto from 'expo-crypto';
import { analyticsStorageKeys, type AnalyticsClient } from '@baukit/analytics-core';
import { HydratedAnalyticsStorage } from '@baukit/analytics-posthog-native/storage';

import { analyticsStoragePrefix, createAnalytics, type ProductEvent } from './analytics-client';

const storageKeys = analyticsStorageKeys(analyticsStoragePrefix);

let analyticsPromise: Promise<AnalyticsClient<ProductEvent>> | undefined;

export function loadAnalytics(): Promise<AnalyticsClient<ProductEvent>> {
  analyticsPromise ??= HydratedAnalyticsStorage.load({
    persistence: AsyncStorage,
    persistentKeys: [storageKeys.anonymousId, storageKeys.userId, storageKeys.aliasedUserId],
  }).then((storage) =>
    createAnalytics(() => Crypto.randomUUID(), __DEV__ ? 'development' : 'production', storage),
  );
  return analyticsPromise;
}
