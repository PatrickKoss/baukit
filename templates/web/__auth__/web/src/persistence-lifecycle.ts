import {
  ScopedPersistenceLifecycle,
  type ClosableScopedPersistence,
  type ScopedPersistenceLifecycleOptions,
} from '@baukit/data-contracts';

export type ProductPersistenceLifecycleOptions<
  TPersistence extends ClosableScopedPersistence,
> = Omit<ScopedPersistenceLifecycleOptions<TPersistence>, 'namespace'>;

/** Product composition seam: validated account identity gates all user-scoped caches. */
export function createProductPersistenceLifecycle<
  TPersistence extends ClosableScopedPersistence,
>(
  options: ProductPersistenceLifecycleOptions<TPersistence>,
): ScopedPersistenceLifecycle<TPersistence> {
  return new ScopedPersistenceLifecycle({
    ...options,
    namespace: '{{ context.app_name }}',
  });
}
