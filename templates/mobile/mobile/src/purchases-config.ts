import { createNoopPurchasesPort, type PurchasesPort } from '@baukit/purchases-core';

export interface PurchasesConfig {
  readonly apiKey: string | null;
  readonly offeringId: string | null;
}

const PLATFORM_KEYS: Readonly<Record<string, string>> = {
  ios: 'iosApiKey',
  android: 'androidApiKey',
};

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() !== '' ? value.trim() : null;
}

function record(value: unknown): Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

export function readPurchasesConfig(extra: unknown, platform: string): PurchasesConfig {
  const revenueCat = record(record(extra)['revenueCat']);
  const keyName = PLATFORM_KEYS[platform];
  return {
    apiKey: keyName === undefined ? null : text(revenueCat[keyName]),
    offeringId: text(revenueCat['offeringId']),
  };
}

export function createPurchasesPort(
  config: PurchasesConfig,
  revenueCat: (apiKey: string) => PurchasesPort,
): PurchasesPort {
  return config.apiKey === null ? createNoopPurchasesPort() : revenueCat(config.apiKey);
}
