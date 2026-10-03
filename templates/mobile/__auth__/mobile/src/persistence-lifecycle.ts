import {
  ScopedPersistenceLifecycle,
  type ClosableScopedPersistence,
  type ScopedPersistenceLifecycleOptions,
} from '@baukit/data-contracts';
import { PRODUCT_NAME } from './product';

export type ProductPersistenceLifecycleOptions<
  TPersistence extends ClosableScopedPersistence,
> = Omit<ScopedPersistenceLifecycleOptions<TPersistence>, 'namespace'>;

/** Product composition seam: the immutable OIDC subject selects storage before repositories mount. */
export function createProductPersistenceLifecycle<
  TPersistence extends ClosableScopedPersistence,
>(
  options: ProductPersistenceLifecycleOptions<TPersistence>,
): ScopedPersistenceLifecycle<TPersistence> {
  return new ScopedPersistenceLifecycle({
    ...options,
    namespace: PRODUCT_NAME,
  });
}
