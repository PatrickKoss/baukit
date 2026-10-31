import { PurchaseError } from './errors.js';
import type { PurchaseOfferings, PurchasesPort } from './port.js';

const EMPTY_OFFERINGS: PurchaseOfferings = { current: null, all: {} };

/** Port for builds without store keys: identity succeeds, nothing is for sale. */
export function createNoopPurchasesPort(): PurchasesPort {
  return {
    identify: () => Promise.resolve(),
    reset: () => Promise.resolve(),
    offerings: () => Promise.resolve(EMPTY_OFFERINGS),
    purchase: () => Promise.reject(new PurchaseError('unavailable')),
    restore: () => Promise.reject(new PurchaseError('unavailable')),
    onCustomerUpdate: () => () => undefined,
  };
}
