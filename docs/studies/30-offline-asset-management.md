# 30. Offline asset management

## Question and scope

Should Baukit extract Eigenruhe's offline download subsystem into a neutral manager with injected manifest, byte source, hash function, metadata store, and file store, plus optional Expo FileSystem and browser Cache Storage adapters? The study also checks Tiefgang, Leitbild, and Redemut for a second asset-heavy consumer and compares the proposal with Baukit's current data, sync, preference, offline-readiness, and local-data ownership contracts. Media production, playback, content metadata, CDN rules, signed URLs, and fallback choices remain outside the manager.

## Evidence table

| Product or Baukit area | File | What it does | What varies or is missing |
| --- | --- | --- | --- |
| Eigenruhe manager | `/home/patrick/projects/eigenruhe/mobile/src/downloads/manager.ts` | `DownloadManager` selects manifest assets, persists progress, resumes partial native downloads, marks old versions stale, deduplicates one active job per selection, and deletes unreferenced hashes. | It depends on the complete product repository type and product manifest fields. Pause is persisted as `queued`. There is no corrupt state, cancel operation, hash check, or cleanup plan. |
| Eigenruhe state | `/home/patrick/projects/eigenruhe/mobile/src/db/types.ts` and `/home/patrick/projects/eigenruhe/mobile/src/db/contracts.ts` | Stores selection fields, status, byte counts, asset hashes, error code, and resume data in the identity-scoped record store. | Status is `queued`, `downloading`, `ready`, `failed`, or `stale`. It lacks `paused`, `complete`, and `corrupt` under the proposed vocabulary. Content item, script, locale, and voice fields are product concepts. |
| Eigenruhe tests | `/home/patrick/projects/eigenruhe/mobile/src/downloads/manager.test.ts` | Tests quota failure, restart from native resume data, stale marking, and deletion. | It does not test duplicate callers, shared-asset concurrency, cancellation, hash mismatch, corrupt cached bytes, cleanup failure, identity switch, or partial cleanup. |
| Eigenruhe native adapter | `/home/patrick/projects/eigenruhe/mobile/src/downloads/platform-file.native.ts` | Uses Expo `DownloadTask`, stores provider resume data, pauses active tasks, checks file size, and maps quota-like messages. | It uses the URL directly and accepts a file when its size matches. It does not hash bytes before returning the URI. Pause applies to every active task. |
| Eigenruhe browser adapter | `/home/patrick/projects/eigenruhe/mobile/src/downloads/platform-file.web.ts` and `/home/patrick/projects/eigenruhe/mobile/src/downloads/platform-file.web.test.ts` | Fetches an asset, writes it to a named Cache Storage cache, creates object URLs, deletes cache entries, and maps HTTP, quota, and missing-cache failures. | It reads global `fetch`, `caches`, and `URL`. It keys by URL and never verifies the response bytes against the manifest hash. Cancellation and resume are absent. |
| Eigenruhe manifest | `/home/patrick/projects/eigenruhe/mobile/src/downloads/manifest.ts` and `/home/patrick/projects/eigenruhe/mobile/src/downloads/manifest-schema.ts` | Validates a product manifest, caches it with an ETag, refreshes every six hours, and retains old item and sound records so stale downloads remain addressable. | The schema includes content item, script, segment, locale, voice, duration, and loop points. Refresh timing and URL policy are product choices. The app schema is maintained separately from the content builder schema. |
| Eigenruhe composition | `/home/patrick/projects/eigenruhe/mobile/src/downloads/provider.tsx` and `/home/patrick/projects/eigenruhe/mobile/src/downloads/clip-resolver.ts` | Connects repository, manifest, network, locale, voice, auto-download, app backgrounding, bundled files, streaming, and playback resolution. | This is product policy. The provider keeps a manager in React state and reuses it when repositories change, so the old identity's repository can remain captured. There is no identity-switch test. |
| Eigenruhe content build | `/home/patrick/projects/eigenruhe/content/README.md` and `/home/patrick/projects/eigenruhe/content/tools/build-manifest.ts` | Builds and validates a manifest with byte counts and SHA-256 digests, then publishes media objects. | Text-to-speech providers, content units, locale, voice, durations, bucket layout, and CDN headers belong to Eigenruhe. Build-time digest creation does not verify downloaded bytes on the device. |
| Redemut content build | `/home/patrick/projects/redemut/backend/crates/redemut-content-compiler/src/lib.rs` and `/home/patrick/projects/redemut/scripts/build-bundled-content.mjs` | Produces deterministic content and audio manifests with hashes and byte counts, then writes product JSON into web and mobile source assets. | This is build-time bundling. It has no runtime download queue, pause, resume, stale state, or cleanup flow. The manifest is learning-specific. |
| Redemut runtime caches | `/home/patrick/projects/redemut/packages/data/src/translation-part-cache.ts`, `/home/patrick/projects/redemut/mobile/src/audio-files.ts`, and `/home/patrick/projects/redemut/mobile/src/reference-audio.tsx` | Dynamically imports bundled translation parts and caches fetched reference audio in Expo's cache directory. It also stores private recordings. | The translation parts ship with the app. Reference audio is fetched on first play, accepted without a manifest hash, and has no durable metadata or cleanup planner. Private recordings have different privacy and retention rules. |
| Tiefgang | `/home/patrick/projects/tiefgang/mobile/src/integrations/files/export-service.ts` and `/home/patrick/projects/tiefgang/mobile/public/sw.js` | Writes user-requested exports and caches an application shell. | It does not consume a remote asset manifest or manage durable offline assets. |
| Leitbild | `/home/patrick/projects/leitbild/web/src/pages/ProgramRunPages.tsx` and `/home/patrick/projects/leitbild/docs/baukit-replay-assessment.md` | Creates a user-requested Markdown export. The repository assessment inventories its web and mobile platform code. | It has no manifest-backed asset downloader, content-addressed file store, or asset lifecycle. |
| Baukit data and identity | `typescript/packages/data-contracts/src/contracts.ts`, `typescript/packages/data-contracts/README.md`, and `docs/platform/local-data-ownership-contract.md` | Provides JSON and record stores, quota normalization, transactions, opaque identity partitions, close-before-open ordering, and stale-work fencing requirements. | It has no byte store, byte-stream source, digest port, or asset state model. Asset files must still follow the existing identity retention policy. |
| Baukit sync, preferences, and offline state | `typescript/packages/sync-client/README.md`, `typescript/packages/preferences-core/README.md`, and `docs/platform/offline-readiness-contract.md` | Covers sync scheduling and outcomes, serialized settings changes, and honest offline readiness. | None owns file integrity, resumable transfers, content-addressed deduplication, or cleanup. Asset activity must not be reported as record sync. |

Redemut is asset-heavy, but it is not a second consumer of the proposed manager. Its bundled content never enters a runtime download state machine, and its reference-audio cache is a single fetch-on-use path without a manifest. Tiefgang and Leitbild also lack such a consumer. Eigenruhe is therefore the only current product that could adopt the whole interface.

Source inspection also contradicts one claim in Eigenruhe's audit. `docs/BAUKIT_FEEDBACK_AUDIT.md` says the app verifies downloaded content hashes, but the native adapter checks only byte count and the browser adapter checks neither byte count nor hash. The build pipeline computes hashes; the runtime does not verify them before read.

## Candidate interface or contract sketch

The evidence is sufficient for a provisional boundary, but not for publication. A future package could use this shape:

```ts
interface OfflineAsset<TMetadata = JsonValue> {
  readonly id: string;
  readonly version: string;
  readonly bytes: number;
  readonly sha256: string;
  readonly metadata: TMetadata;
}

interface OfflineAssetUnit<TMetadata = JsonValue> {
  readonly key: string;
  readonly assets: readonly OfflineAsset<TMetadata>[];
}

interface OfflineAssetManifest<TMetadata = JsonValue> {
  readonly version: string;
  readonly units: readonly OfflineAssetUnit<TMetadata>[];
}

type OfflineAssetStatus =
  | "queued"
  | "downloading"
  | "paused"
  | "complete"
  | "stale"
  | "corrupt"
  | "failed";

interface OfflineAssetRecord {
  readonly unitKey: string;
  readonly manifestVersion: string;
  readonly status: OfflineAssetStatus;
  readonly bytesTotal: number;
  readonly bytesDone: number;
  readonly fileKeys: readonly string[];
  readonly resumeTokens: Readonly<Record<string, string>>;
  readonly errorCode: string | null;
}

interface AssetByteSource {
  open(asset: OfflineAsset, signal: AbortSignal): Promise<AsyncIterable<Uint8Array>>;
}

interface AssetHasher {
  createSha256(): {
    update(bytes: Uint8Array): void;
    digest(): Promise<string>;
  };
}

interface StoredAssetFile {
  readonly fileKey: string;
  readonly uri: string;
  read(): AsyncIterable<Uint8Array>;
}

interface AssetFileWriter {
  append(bytes: Uint8Array): Promise<void>;
  checkpoint(): Promise<string | null>;
  commit(): Promise<StoredAssetFile>;
  discard(): Promise<void>;
}

interface AssetFileStore {
  begin(asset: OfflineAsset, resumeToken: string | null): Promise<AssetFileWriter>;
  inspect(asset: OfflineAsset): Promise<StoredAssetFile | undefined>;
  remove(fileKey: string): Promise<void>;
}

interface AssetMetadataStore {
  get(unitKey: string): Promise<OfflineAssetRecord | undefined>;
  list(): Promise<readonly OfflineAssetRecord[]>;
  put(record: OfflineAssetRecord): Promise<void>;
  delete(unitKey: string): Promise<void>;
}

interface AssetCleanupPlan {
  readonly generation: string;
  readonly unitKeys: readonly string[];
  readonly fileKeys: readonly string[];
  readonly reclaimableBytes: number;
}

interface AssetCleanupOutcome {
  readonly removedUnitKeys: readonly string[];
  readonly removedFileKeys: readonly string[];
  readonly failedUnitKeys: readonly string[];
  readonly failedFileKeys: readonly string[];
}

interface OfflineAssetManager<TMetadata = JsonValue> {
  setManifest(manifest: OfflineAssetManifest<TMetadata>): Promise<void>;
  queue(unitKey: string): Promise<OfflineAssetRecord>;
  pause(unitKey: string): Promise<void>;
  cancel(unitKey: string): Promise<void>;
  resolve(assetId: string, version: string): Promise<string | undefined>;
  planCleanup(keepUnitKeys: ReadonlySet<string>): Promise<AssetCleanupPlan>;
  executeCleanup(plan: AssetCleanupPlan): Promise<AssetCleanupOutcome>;
  resetIdentity(metadataStore: AssetMetadataStore, fileStore: AssetFileStore): Promise<void>;
  subscribe(listener: () => void): () => void;
}
```

The writer has explicit append, commit, discard, and resumable-token operations. The manager hashes each chunk while writing and calls `commit` only after the digest matches the manifest. A temporary file or cache entry remains unreadable until commit. `resolve` reads through `inspect` and hashes the committed bytes before returning the URI. An adapter may skip that full read only when it can prove that its integrity receipt cannot outlive or become detached from the exact stored bytes. Hash and size comparison use manifest values, not file extensions or URLs. Cleanup execution rejects a stale generation and rechecks references before each deletion.

Stable error codes should be limited to `manifest_invalid`, `source_unavailable`, `quota_exceeded`, `integrity_mismatch`, `storage_unavailable`, `cancelled`, and `cleanup_failed`. Error values must not contain signed URLs, response bodies, identity keys, media bytes, or product metadata. The browser adapter should receive `fetch`, `caches`, and object-URL operations by injection. The Expo adapter may use Expo FileSystem only through an optional entry point.

The package must not choose a fallback. A product calls `resolve`, then applies its own bundled, alternate-locale, streaming, or unavailable policy.

## Required-case coverage

| Required case | Coverage today | Required contract or missing proof |
| --- | --- | --- |
| Queued | Eigenruhe persists `queued` before download. | Keep it as an observable durable state. |
| Downloading | Eigenruhe persists `downloading` and byte progress. | Bound progress writes and define behavior when metadata persistence fails after bytes advance. |
| Paused | Expo tasks return resume data, but the manager persists the state as `queued`. | Add a distinct durable `paused` state and test restart with a valid and corrupt resume token. |
| Complete | Eigenruhe uses `ready` after every asset resolves. | Name the neutral state `complete` and require verified commits for every asset. |
| Stale | Eigenruhe marks an older ready script version stale while leaving its files resolvable. | Define staleness through caller manifest identity rather than numeric script versions. |
| Corrupt | No runtime path detects corrupt bytes. | Hash mismatch must delete or quarantine the temporary result, persist `corrupt`, and never return a URI. |
| Failed | Eigenruhe maps quota and download failures into a persisted failed row. | Preserve stable codes and the prior verified version. Do not store provider messages in durable metadata. |
| Cancellation | Eigenruhe can pause every active native task when the app backgrounds. It cannot cancel one unit. | Cancel one unit, abort its byte source and writer, settle duplicate callers, and state whether resumable bytes remain. |
| In-flight deduplication | Eigenruhe deduplicates one selection key in `#jobs`. There is no direct test. Two selections that share a hash can still start duplicate asset downloads. | Test same-unit callers and different units sharing one asset. The asset digest and version should identify one active transfer. |
| Hash before read | The content build computes hashes. Native `resolve` checks file size only; web `resolve` trusts the URL cache entry. | Verify bytes before commit and before first read after untrusted recovery. Size equality is insufficient. |
| Cleanup planning separate from deletion | Eigenruhe computes references and deletes in the same loop. | Return a stable plan first. Execution reports each file and metadata outcome so partial failure can be retried without deleting a shared file. |
| Identity change | Baukit mounts identity-scoped repositories, but Eigenruhe's provider can retain a manager that captured the previous repository. | Stop old transfers, detach listeners, fence late progress, switch metadata and file ownership, then publish the new snapshot. Test A to B to A under the chosen retain or delete policy. |
| Caller-owned fallback policy | Eigenruhe's clip resolver tries a verified-status download, then a bundled clip, then network streaming. Locale and voice selection happen in its provider. | Keep the manager result neutral. Test only that no fallback runs inside the manager. |

A future implementation also needs offline-start recovery, stale-manifest behavior, source interruption, browser quota errors, partial cleanup, manifest change during download, and artifact tests against real Expo FileSystem and browser Cache Storage. `@baukit/data-contracts` can supply metadata persistence, but the file and byte adapters need their own conformance suite.

## Decision

Decision: defer until a second product has a manifest-backed runtime download flow with pause or cancellation, integrity checking, durable metadata, and cleanup. Redemut is the closest asset-heavy product, but its bundled content and fetch-on-play reference audio do not exercise this interface. The smallest next step is in Eigenruhe: unify its build and app manifest schema, add runtime hash verification, represent paused and corrupt states, separate cleanup planning, and fix identity replacement. Those changes will show which parts survive real failure handling. Revisit the package when another product can name code that the same manager would delete.

## What stays product-owned

- Content units and metadata such as scripts, segments, locales, voices, durations, loop points, and dependencies.
- Manifest transport, refresh interval, ETag use, CDN origin, authentication, signed-URL renewal, and retry timing.
- Automatic-download eligibility, storage estimates shown to users, network policy, and copy.
- Bundled starter assets, locale and voice fallback, streaming decisions, playback, keep-awake behavior, and audio provider code.
- Identity retention and erasure policy. The manager must follow the existing local-data ownership contract and use the active product partition.
- Media compilation, text-to-speech tools, codecs, normalization, upload tools, object-store layout, and provider SDKs. None belong in baseline projects.
