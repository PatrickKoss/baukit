# 30. Offline asset management

## Source product files

- `/home/patrick/projects/eigenruhe/mobile/src/downloads/manager.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/manager.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/file-port.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/platform-file.native.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/platform-file.web.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/manifest.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/manifest-schema.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/provider.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/downloads/clip-resolver.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/db/types.ts`
- `/home/patrick/projects/eigenruhe/content/tools/build-manifest.ts`
- `/home/patrick/projects/redemut/backend/crates/redemut-content-compiler/src/lib.rs`
- `/home/patrick/projects/redemut/scripts/build-bundled-content.mjs`
- `/home/patrick/projects/redemut/packages/data/src/translation-part-cache.ts`
- `/home/patrick/projects/redemut/mobile/src/audio-files.ts`
- `/home/patrick/projects/redemut/mobile/src/reference-audio.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/integrations/files/export-service.ts`
- `/home/patrick/projects/tiefgang/mobile/public/sw.js`
- `/home/patrick/projects/leitbild/web/src/pages/ProgramRunPages.tsx`

## Observed failure or repeated glue

Eigenruhe is the only manifest-backed offline downloader. It has useful queue, resume, stale, progress, and reference-counted deletion mechanics, but no runtime SHA-256 verification, corrupt state, per-unit cancellation, separate cleanup plan, or identity-switch fence. Native resolution trusts byte count and browser resolution trusts a URL-keyed cache entry. Redemut is asset-heavy but ships content as build assets; its reference audio cache is fetch-on-play without manifest integrity or durable lifecycle. Tiefgang and Leitbild have exports and application-shell caching, not offline asset managers.

## Baukit owner

No Baukit package owns this capability now. If the evidence gate is met, an optional `@baukit/offline-assets` package should own the neutral manager and conformance suite. Optional `@baukit/offline-assets/expo` and `@baukit/offline-assets/browser` entries would own platform adapters. These packages must not enter baseline products that do not select the capability.

## Public types and errors

The provisional sketch names `OfflineAsset`, `OfflineAssetUnit`, `OfflineAssetManifest`, `OfflineAssetStatus`, `OfflineAssetRecord`, `AssetByteSource`, `AssetHasher`, `StoredAssetFile`, `AssetFileWriter`, `AssetFileStore`, `AssetMetadataStore`, `AssetCleanupPlan`, `AssetCleanupOutcome`, and `OfflineAssetManager`. Stable error codes are `manifest_invalid`, `source_unavailable`, `quota_exceeded`, `integrity_mismatch`, `storage_unavailable`, `cancelled`, and `cleanup_failed`. No public error contains a URL, response body, identity, media byte, or caller metadata.

## Product-owned inputs

Products supply decoded manifest units and metadata, byte-source authorization, hash implementation where the platform lacks one, identity-scoped stores, content grouping, dependencies, stale-version meaning, network and retry policy, quota copy, cleanup selection, fallback, locale and voice choice, CDN and signed URLs, playback, and media tooling.

## Concurrency, failure, privacy, and cleanup cases

Required tests cover every state, restart from valid and corrupt resume data, cancellation, duplicate callers, shared hashes across units, interruption, manifest replacement during transfer, hash mismatch, corrupt committed bytes, quota and metadata failures, offline startup, late progress after identity change, A to B to A retention, cleanup planning, shared-file protection, and partial cleanup retry. Temporary bytes stay unreadable until hash verification. Errors, logs, state, and metrics omit signed URLs, source bodies, identities, and product metadata. Cleanup planning performs no deletion.

## Supported runtimes

No runtime support is claimed while the item is deferred. A future root package should be framework-free ES2022 for browser, React Native, and Node tests. The Expo adapter must be tested on the supported Android and iOS Expo FileSystem versions. The browser adapter must be tested in real supported browsers with Cache Storage, streaming, abort, quota, and object-URL cleanup.

## Product adoption change

There is no second adoption target. If the evidence gate is later met, Eigenruhe could delete `mobile/src/downloads/manager.ts`, `file-port.ts`, `platform-file.native.ts`, and `platform-file.web.ts`. Its provider, manifest mapping, fallback resolver, content metadata, copy, and playback stay local. Redemut, Tiefgang, and Leitbild have no current file that the same manager can replace.

## Throwaway experiments

None. The study used source inspection only.
