import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js';
import type { ZodType } from 'zod';

import type { ApiClient, OpenApiRoute } from '../api/client.js';

export interface RequiredToolAnnotations {
  readonly destructiveHint: boolean;
  readonly idempotentHint: boolean;
  readonly openWorldHint: boolean;
  readonly readOnlyHint: boolean;
}

export interface ToolDefinition {
  readonly annotations: RequiredToolAnnotations;
  readonly description: string;
  readonly inputSchema: ZodType;
  readonly name: string;
  readonly route: OpenApiRoute;
  readonly run: (api: ApiClient) => Promise<unknown>;
}

export type ToolLogger = (event: {
  readonly name: string;
  readonly outcome: 'error' | 'success';
  readonly status?: number;
}) => void;

export function jsonResult(value: unknown): CallToolResult {
  return {
    content: [{ type: 'text', text: JSON.stringify(value) }],
  };
}
