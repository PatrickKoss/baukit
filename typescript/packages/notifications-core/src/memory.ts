import type {
  NotificationPermission,
  NotificationPlatform,
  OwnedNotificationScheduleRequest,
  PendingNotification,
} from './scheduler.js';

export interface NotificationPlatformFaults {
  setPermission(permission: NotificationPermission): void;
  failNextList(): void;
  failNextPermission(): void;
  failCancel(identifier: string): void;
  failSchedule(identifier: string): void;
  holdNextList(): () => void;
  afterNextList(action: () => void): void;
}

export interface StoredNotification<TContent> extends PendingNotification {
  readonly content: TContent | null;
}

export class NotificationPlatformFaultState implements NotificationPlatformFaults {
  permission: NotificationPermission = 'granted';
  private listFailures = 0;
  private permissionFailures = 0;
  private readonly cancelFailures = new Set<string>();
  private readonly scheduleFailures = new Set<string>();
  private readonly listHolds: Promise<void>[] = [];
  private readonly listActions: (() => void)[] = [];

  setPermission(permission: NotificationPermission): void {
    this.permission = permission;
  }

  failNextList(): void {
    this.listFailures += 1;
  }

  failNextPermission(): void {
    this.permissionFailures += 1;
  }

  failCancel(identifier: string): void {
    this.cancelFailures.add(identifier);
  }

  failSchedule(identifier: string): void {
    this.scheduleFailures.add(identifier);
  }

  holdNextList(): () => void {
    let release = (): void => undefined;
    this.listHolds.push(
      new Promise<void>((resolve) => {
        release = resolve;
      }),
    );
    return release;
  }

  afterNextList(action: () => void): void {
    this.listActions.push(action);
  }

  async beforeList(): Promise<void> {
    await this.listHolds.shift();
    if (this.listFailures > 0) {
      this.listFailures -= 1;
      throw new Error('list failed');
    }
  }

  afterList(): void {
    this.listActions.shift()?.();
  }

  beforePermission(): void {
    if (this.permissionFailures > 0) {
      this.permissionFailures -= 1;
      throw new Error('permission check failed');
    }
  }

  beforeCancel(identifier: string): void {
    if (this.cancelFailures.delete(identifier)) {
      throw new Error('cancel failed');
    }
  }

  beforeSchedule(identifier: string): void {
    if (this.scheduleFailures.delete(identifier)) {
      throw new Error('schedule failed');
    }
  }
}

export class InMemoryNotificationPlatform<TContent> implements NotificationPlatform<TContent> {
  readonly faults = new NotificationPlatformFaultState();
  private readonly requests = new Map<string, StoredNotification<TContent>>();

  addUnrelated(identifier: string): void {
    this.requests.set(identifier, { identifier, marker: null, content: null });
  }

  pending(): readonly StoredNotification<TContent>[] {
    return [...this.requests.values()].sort((left, right) =>
      left.identifier < right.identifier ? -1 : 1,
    );
  }

  async list(): Promise<readonly PendingNotification[]> {
    await this.faults.beforeList();
    const snapshot = this.pending().map(({ identifier, marker }) => ({ identifier, marker }));
    this.faults.afterList();
    return snapshot;
  }

  cancel(identifier: string): Promise<void> {
    return settle(() => {
      this.faults.beforeCancel(identifier);
      this.requests.delete(identifier);
    });
  }

  permission(): Promise<NotificationPermission> {
    return settle(() => {
      this.faults.beforePermission();
      return this.faults.permission;
    });
  }

  schedule(request: OwnedNotificationScheduleRequest<TContent>): Promise<void> {
    return settle(() => {
      this.faults.beforeSchedule(request.identifier);
      this.requests.set(request.identifier, {
        identifier: request.identifier,
        marker: request.marker,
        content: request.content,
      });
    });
  }
}

function settle<T>(action: () => T): Promise<T> {
  return new Promise((resolve) => {
    resolve(action());
  });
}
