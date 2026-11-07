import type {
  SyncConformanceAdapter,
  SyncConformanceFullResyncCallbacks,
  SyncConformanceLocalApplyOptions,
  SyncConformancePushResult,
  SyncConformanceScenario,
  SyncConformanceStaleCursor,
} from './conformance.js';
import { assert, fail, PAGE_SIZE, type TestContext } from './conformance-support.js';
import { validatePullPage } from './transport.js';

export interface SyncConformanceSubmission<TPending, TPushResponse, TRemoteChange> {
  readonly submitted: readonly TPending[];
  readonly response: TPushResponse;
  readonly outcome: SyncConformancePushResult<TPending, TRemoteChange>;
}

/** Scenario helpers shared by every conformance case. */
export class SyncConformanceKit<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
> {
  public constructor(
    public readonly adapter: SyncConformanceAdapter<
      TClient,
      TServer,
      TPending,
      TPushRequest,
      TPushResponse,
      TPullResponse,
      TRemoteChange,
      TCursor
    >,
  ) {}

  public readonly inScenario = async (
    run: (context: TestContext<TClient, TServer>) => Promise<void>,
  ): Promise<void> => {
    const scenario: SyncConformanceScenario<TClient, TServer> = await this.adapter.createScenario();
    const [clientA, clientB] = scenario.clients;
    try {
      await run({ clientA, clientB, server: scenario.server });
    } finally {
      await this.adapter.disposeScenario?.(scenario);
    }
  };

  public readonly pending = async (client: TClient): Promise<readonly TPending[]> =>
    this.adapter.outbox.listPending(client);

  public readonly assertPendingState = async (
    client: TClient,
    expectedCount: number,
  ): Promise<void> => {
    const actual = await this.pending(client);
    const reported = await this.adapter.local.readPendingState(client);
    assert(
      actual.length === expectedCount,
      `expected ${String(expectedCount)} pending changes, received ${String(actual.length)}`,
    );
    assert(
      reported.pendingCount === actual.length,
      'reported pending count differs from the outbox',
    );
    assert(reported.pending === actual.length > 0, 'reported pending flag differs from the outbox');
  };

  public readonly submit = async (
    client: TClient,
    server: TServer,
  ): Promise<SyncConformanceSubmission<TPending, TPushResponse, TRemoteChange>> => {
    const submitted = await this.pending(client);
    assert(submitted.length > 0, 'cannot submit an empty pending batch');
    const request = await this.adapter.wire.encodePush(submitted);
    const response = await this.adapter.server.acceptPush(server, request);
    const outcome = await this.adapter.wire.decodePush(response, submitted);
    return { submitted, response, outcome };
  };

  public readonly applySubmittedBatchOutcome = async (
    client: TClient,
    submission: SyncConformanceSubmission<TPending, TPushResponse, TRemoteChange>,
  ): Promise<void> => {
    await this.adapter.local.applySubmittedBatchOutcome(client, {
      submitted: submission.submitted,
      response: submission.response,
      ...submission.outcome,
    });
  };

  public readonly push = async (client: TClient, server: TServer): Promise<void> => {
    const submitted = await this.pending(client);
    if (submitted.length === 0) return;
    await this.applySubmittedBatchOutcome(client, await this.submit(client, server));
  };

  public readonly pullPage = async (
    client: TClient,
    server: TServer,
    options?: SyncConformanceLocalApplyOptions,
  ): Promise<boolean> => {
    const cursor = (await this.adapter.local.readCursor(client)) as TCursor;
    const response = await this.adapter.server.servePull(server, cursor, PAGE_SIZE);
    const page = await this.adapter.wire.decodePull(response);
    validatePullPage(cursor, page, (left, right) => this.adapter.wire.compareCursors(left, right));
    await this.adapter.local.applyPullPage(client, page, options);
    return page.hasMore;
  };

  public readonly pullAll = async (client: TClient, server: TServer): Promise<void> => {
    let hasMore = true;
    while (hasMore) hasMore = await this.pullPage(client, server);
  };

  public readonly expectStalePull = async (
    client: TClient,
    server: TServer,
    fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
  ): Promise<SyncConformanceStaleCursor<TCursor>> => {
    const cursor = await this.adapter.local.readCursor(client);
    try {
      await this.adapter.server.servePull(server, cursor, PAGE_SIZE);
    } catch (error) {
      const stale = fullResync.decodeStaleCursor(error);
      if (stale === null) throw error;
      assert(
        this.adapter.wire.compareCursors(stale.horizon, cursor) > 0,
        'stale cursor response did not contain a newer purge horizon',
      );
      return stale;
    }
    fail('stale cursor returned a pull page');
  };

  public readonly recoverFromStaleCursor = async (
    client: TClient,
    server: TServer,
    fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
  ): Promise<'completed' | 'deferred'> => {
    const stale = await this.expectStalePull(client, server, fullResync);
    if (!(await fullResync.isResetSafe(client, stale))) return 'deferred';
    await fullResync.reset(client, stale);
    assert(
      this.adapter.wire.compareCursors(
        await this.adapter.local.readCursor(client),
        fullResync.zeroCursor,
      ) === 0,
      'full-resync reset did not store cursor zero',
    );
    await this.pullAll(client, server);
    return 'completed';
  };
}
