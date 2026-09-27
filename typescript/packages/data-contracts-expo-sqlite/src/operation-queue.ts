/** Runs operations one at a time in call order. A rejected operation releases the queue. */
export class OperationQueue {
  private tail: Promise<void> = Promise.resolve();
  private pending = 0;

  public constructor(private readonly onIdle: () => void = () => undefined) {}

  public run<TResult>(operation: () => Promise<TResult>): Promise<TResult> {
    this.pending += 1;
    const result = this.tail.then(operation);
    const settle = (): void => {
      this.settle();
    };
    this.tail = result.then(settle, settle);
    return result;
  }

  private settle(): void {
    this.pending -= 1;
    if (this.pending === 0) {
      this.onIdle();
    }
  }
}

const fileQueues = new Map<string, OperationQueue>();

/**
 * Returns the queue shared by every adapter statement against one SQLite file.
 * An entry exists only while work is pending, so closed databases leave nothing behind.
 */
export function queueForFile(databasePath: string): OperationQueue {
  const existing = fileQueues.get(databasePath);
  if (existing !== undefined) {
    return existing;
  }
  const queue = new OperationQueue(() => {
    fileQueues.delete(databasePath);
  });
  fileQueues.set(databasePath, queue);
  return queue;
}
