import Constants from 'expo-constants';
import { Platform } from 'react-native';
import Purchases from 'react-native-purchases';
import { PurchaseFlow, type EntitlementBackend } from '@baukit/purchases-core';
import { createRevenueCatPurchasesPort } from '@baukit/purchases-revenuecat';

import { createPurchasesPort, readPurchasesConfig } from './purchases-config';

export const purchasesConfig = readPurchasesConfig(Constants.expoConfig?.extra, Platform.OS);

// Replace with createAccountPlanBackend once the backend mounts account_plan_router.
const standaloneBackend: EntitlementBackend = {
  refreshEntitlement: () => Promise.resolve(null),
};

let flow: PurchaseFlow | undefined;

export function purchaseFlow(): PurchaseFlow {
  flow ??= new PurchaseFlow({
    port: createPurchasesPort(purchasesConfig, (apiKey) =>
      createRevenueCatPurchasesPort(Purchases, { apiKey }),
    ),
    backend: standaloneBackend,
  });
  return flow;
}
