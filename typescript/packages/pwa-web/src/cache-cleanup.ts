export interface CacheCleanupPorts {
  readonly listCacheNames: () => Promise<readonly string[]>;
  readonly deleteCache: (name: string) => Promise<boolean>;
}

export interface CacheCleanupOptions {
  readonly ports: CacheCleanupPorts;
  readonly shouldDelete: (name: string) => boolean;
}

export interface CacheCleanupResult {
  readonly matchedCount: number;
  readonly deletedCount: number;
}

export async function cleanupCaches(options: CacheCleanupOptions): Promise<CacheCleanupResult> {
  const names = await options.ports.listCacheNames();
  const selected = names.filter(options.shouldDelete);
  const deleted = await Promise.all(selected.map((name) => options.ports.deleteCache(name)));

  return {
    matchedCount: selected.length,
    deletedCount: deleted.filter(Boolean).length,
  };
}
