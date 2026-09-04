import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createServer } from '../src/server.js';
import { READ_TOOLS } from '../src/tools/read.js';
import { WRITE_TOOLS } from '../src/tools/write.js';

const closeConnections: Array<() => Promise<void>> = [];

afterEach(async () => {
  await Promise.all(closeConnections.splice(0).map((close) => close()));
});

describe('MCP server', () => {
  it('registers annotated read tools before write tools', async () => {
    const { client } = await connect(() =>
      Promise.resolve(jsonResponse(200, [])),
    );
    const listed = await client.listTools();

    expect(listed.tools.map(({ name }) => name)).toEqual([
      ...READ_TOOLS.map(({ name }) => name),
      ...WRITE_TOOLS.map(({ name }) => name),
    ]);
    expect(READ_TOOLS).toHaveLength(1);
    expect(WRITE_TOOLS).toHaveLength(0);
    for (const tool of listed.tools) {
      expect(tool.annotations).toMatchObject({
        readOnlyHint: true,
        destructiveHint: false,
        idempotentHint: true,
        openWorldHint: false,
      });
    }
  });

  it('rejects malformed tool input before calling the API', async () => {
    const apiFetch = vi.fn(() => Promise.resolve(jsonResponse(200, [])));
    const { client } = await connect(apiFetch);
    const result = await client.callTool({
      name: 'example_list_items',
      arguments: { unexpected: true },
    });

    expect(result.isError).toBe(true);
    expect(apiFetch).not.toHaveBeenCalled();
  });

  it('converts backend failures without returning or logging their body', async () => {
    const credential = 'private-test-token';
    const logger = vi.fn();
    const { client } = await connect(
      () => Promise.resolve(jsonResponse(500, { exception: credential })),
      () => credential,
      logger,
    );
    const result = (await client.callTool({
      name: 'example_list_items',
      arguments: {},
    })) as CallToolResult;
    const rendered = JSON.stringify(result);
    const logged = JSON.stringify(logger.mock.calls);

    expect(result.isError).toBe(true);
    expect(rendered).toContain('api_error');
    expect(rendered).toContain('500');
    expect(rendered).not.toContain(credential);
    expect(logged).not.toContain(credential);
    expect(logged).toContain('error');
    await expect(client.ping()).resolves.toEqual({});
  });

  it('returns a fixed auth failure and does not call fetch', async () => {
    const apiFetch = vi.fn(() => Promise.resolve(jsonResponse(200, [])));
    const logger = vi.fn();
    const { client } = await connect(
      apiFetch,
      () => Promise.reject(new Error('credential detail')),
      logger,
    );
    const result = (await client.callTool({
      name: 'example_list_items',
      arguments: {},
    })) as CallToolResult;

    expect(result.isError).toBe(true);
    expect(JSON.stringify(result)).toContain('authentication_failed');
    expect(JSON.stringify(result)).not.toContain('credential detail');
    expect(JSON.stringify(logger.mock.calls)).not.toContain(
      'credential detail',
    );
    expect(apiFetch).not.toHaveBeenCalled();
  });

  it('rejects an oversized response without exposing its contents', async () => {
    const privateBody = `private-${'x'.repeat(1024 * 1024)}`;
    const { client } = await connect(() =>
      Promise.resolve(jsonResponse(200, { value: privateBody })),
    );
    const result = (await client.callTool({
      name: 'example_list_items',
      arguments: {},
    })) as CallToolResult;

    expect(result.isError).toBe(true);
    expect(JSON.stringify(result)).toContain('api_error');
    expect(JSON.stringify(result)).not.toContain('private-');
  });
});

async function connect(
  apiFetch: typeof globalThis.fetch,
  bearerToken: () => string | Promise<string> = () => 'test-token',
  logger: Parameters<typeof createServer>[0]['logger'] = () => undefined,
) {
  const server = createServer({
    apiUrl: 'http://api.test',
    bearerToken,
    fetch: apiFetch,
    logger,
  });
  const client = new Client({ name: 'fixture-test', version: '1.0.0' });
  const [clientTransport, serverTransport] =
    InMemoryTransport.createLinkedPair();
  await Promise.all([
    server.connect(serverTransport),
    client.connect(clientTransport),
  ]);
  closeConnections.push(async () => {
    await client.close();
    await server.close();
  });
  return { client };
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}
