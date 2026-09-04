import { describe, expect, it } from 'vitest';

import type { ImportEnvelopeSource } from './import-envelope.js';

export interface ImportEnvelopeConformanceFixtures {
  readonly valid: ImportEnvelopeSource;
  readonly unknownFields: ImportEnvelopeSource;
  readonly duplicateIds: ImportEnvelopeSource;
  readonly tombstones: ImportEnvelopeSource;
  readonly ownershipAndRevisionFields: ImportEnvelopeSource;
  readonly oversizedStrings: ImportEnvelopeSource;
  readonly excessRows: ImportEnvelopeSource;
  readonly mixedVersions: ImportEnvelopeSource;
  readonly failureHalfwayThroughCommit: ImportEnvelopeSource;
}

export interface ImportEnvelopeConformanceHarnessOptions {
  readonly failAfterWrites?: number;
}

export interface ImportEnvelopeConformanceObservation<TCursor, TSyncState> {
  readonly committedIds: readonly string[];
  readonly writeFields: readonly (readonly string[])[];
  readonly cursor: TCursor;
  readonly syncState: TSyncState;
  readonly stateAdvancedBeforeCommit: boolean;
}

export interface ImportEnvelopeConformanceHarness<TPreview, TCursor, TSyncState> {
  preview(source: ImportEnvelopeSource): Promise<TPreview>;
  commit(preview: TPreview): Promise<void>;
  observe():
    | ImportEnvelopeConformanceObservation<TCursor, TSyncState>
    | Promise<ImportEnvelopeConformanceObservation<TCursor, TSyncState>>;
}

export interface ImportEnvelopeConformanceOptions<TPreview, TCursor, TSyncState> {
  readonly fixtures: ImportEnvelopeConformanceFixtures;
  readonly makeHarness: (
    options: ImportEnvelopeConformanceHarnessOptions,
  ) =>
    | ImportEnvelopeConformanceHarness<TPreview, TCursor, TSyncState>
    | Promise<ImportEnvelopeConformanceHarness<TPreview, TCursor, TSyncState>>;
  readonly expectedCommittedIds: readonly string[];
  readonly forbiddenWriteFields: readonly string[];
  readonly initialCursor: TCursor;
  readonly committedCursor: TCursor;
  readonly initialSyncState: TSyncState;
  readonly committedSyncState: TSyncState;
}

/** Registers import validation, preview, atomicity, and post-commit state requirements. */
export function describeImportEnvelopeContract<TPreview, TCursor, TSyncState>(
  options: ImportEnvelopeConformanceOptions<TPreview, TCursor, TSyncState>,
): void {
  describe('import-envelope contract', () => {
    const rejectedFixtures = [
      ['unknown fields', options.fixtures.unknownFields],
      ['duplicate IDs', options.fixtures.duplicateIds],
      ['tombstones', options.fixtures.tombstones],
      ['ownership and revision fields', options.fixtures.ownershipAndRevisionFields],
      ['oversized strings', options.fixtures.oversizedStrings],
      ['excess rows', options.fixtures.excessRows],
      ['mixed versions', options.fixtures.mixedVersions],
    ] as const;

    it.each(rejectedFixtures)('rejects %s before a product write', async (_name, source) => {
      const harness = await options.makeHarness({});
      await expect(harness.preview(source)).rejects.toThrow();
      const observed = await harness.observe();
      expect(observed.committedIds).toEqual([]);
      expect(observed.writeFields).toEqual([]);
      expect(observed.cursor).toEqual(options.initialCursor);
      expect(observed.syncState).toEqual(options.initialSyncState);
    });

    it('previews without writing or advancing state', async () => {
      const harness = await options.makeHarness({});
      await expect(harness.preview(options.fixtures.valid)).resolves.toBeDefined();
      const observed = await harness.observe();
      expect(observed.committedIds).toEqual([]);
      expect(observed.writeFields).toEqual([]);
      expect(observed.cursor).toEqual(options.initialCursor);
      expect(observed.syncState).toEqual(options.initialSyncState);
    });

    it('commits every row before advancing cursor and sync state', async () => {
      const harness = await options.makeHarness({});
      const preview = await harness.preview(options.fixtures.valid);
      await expect(harness.commit(preview)).resolves.toBeUndefined();
      const observed = await harness.observe();
      expect(observed.committedIds).toEqual(options.expectedCommittedIds);
      expect(observed.writeFields).toHaveLength(options.expectedCommittedIds.length);
      expect(observed.cursor).toEqual(options.committedCursor);
      expect(observed.syncState).toEqual(options.committedSyncState);
      expect(observed.stateAdvancedBeforeCommit).toBe(false);
      for (const fields of observed.writeFields) {
        for (const forbidden of options.forbiddenWriteFields) {
          expect(fields).not.toContain(forbidden);
        }
      }
    });

    it('rolls back every row and leaves state unchanged after a halfway failure', async () => {
      const harness = await options.makeHarness({ failAfterWrites: 1 });
      const preview = await harness.preview(options.fixtures.failureHalfwayThroughCommit);
      await expect(harness.commit(preview)).rejects.toThrow();
      const observed = await harness.observe();
      expect(observed.committedIds).toEqual([]);
      expect(observed.cursor).toEqual(options.initialCursor);
      expect(observed.syncState).toEqual(options.initialSyncState);
      expect(observed.stateAdvancedBeforeCommit).toBe(false);
    });
  });
}
