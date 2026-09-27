export type NotificationPlanErrorCode =
  | 'invalid_civil_date'
  | 'invalid_civil_time'
  | 'invalid_time_zone'
  | 'invalid_horizon'
  | 'invalid_clock'
  | 'invalid_namespace'
  | 'invalid_logical_id'
  | 'invalid_content_digest'
  | 'invalid_instant'
  | 'invalid_pending_limit'
  | 'duplicate_logical_id'
  | 'reserved_data_key';

export class NotificationPlanError extends Error {
  override readonly name = 'NotificationPlanError';
  readonly code: NotificationPlanErrorCode;
  readonly logicalId: string | undefined;

  constructor(code: NotificationPlanErrorCode, logicalId?: string) {
    super(logicalId === undefined ? code : `${code}: ${logicalId}`);
    this.code = code;
    this.logicalId = logicalId;
  }
}
