import type { OpenApiRoute } from './api/client.js';

export const PRODUCT_TOOL_ROUTES = {
  example_list_items: { method: 'get', path: '/items' },
} as const satisfies Readonly<Record<string, OpenApiRoute>>;
