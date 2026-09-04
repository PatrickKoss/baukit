import { z } from 'zod';

import { PRODUCT_TOOL_ROUTES } from '../tool-routes.js';
import type { ToolDefinition } from './registry.js';

export const READ_TOOLS = [
  {
    name: 'example_list_items',
    description:
      'Lists the example items. Replace this tool while keeping the registry contract.',
    inputSchema: z.object({}).strict(),
    annotations: {
      readOnlyHint: true,
      destructiveHint: false,
      idempotentHint: true,
      openWorldHint: false,
    },
    route: PRODUCT_TOOL_ROUTES.example_list_items,
    run: (api) => api.request(PRODUCT_TOOL_ROUTES.example_list_items),
  },
] as const satisfies readonly ToolDefinition[];
