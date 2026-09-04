import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { READ_TOOLS } from '../dist/tools/read.js';
import { PRODUCT_TOOL_ROUTES } from '../dist/tool-routes.js';
import { WRITE_TOOLS } from '../dist/tools/write.js';

const root = new URL('../../', import.meta.url);
const manifest = await readFile(new URL('baukit.toml', root), 'utf8');
const schemaMatch = /^schema\s*=\s*"([^"]+)"\s*$/mu.exec(manifest);
if (schemaMatch?.[1] === undefined)
  throw new Error('baukit.toml has no OpenAPI schema path.');
const schemaUrl = process.env['MCP_OPENAPI_SCHEMA']
  ? pathToFileURL(resolve(process.env['MCP_OPENAPI_SCHEMA']))
  : new URL(schemaMatch[1], root);
const schema = JSON.parse(await readFile(schemaUrl, 'utf8'));
const tools = [...READ_TOOLS, ...WRITE_TOOLS];
const failures = [];

for (const tool of tools) {
  const allowed = PRODUCT_TOOL_ROUTES[tool.name];
  if (allowed === undefined) {
    failures.push(`${tool.name}: missing from PRODUCT_TOOL_ROUTES`);
    continue;
  }
  if (
    allowed.method !== tool.route.method ||
    allowed.path !== tool.route.path
  ) {
    failures.push(
      `${tool.name}: registry route differs from PRODUCT_TOOL_ROUTES`,
    );
  }
  if (schema.paths?.[allowed.path]?.[allowed.method] === undefined) {
    failures.push(
      `${tool.name}: ${allowed.method.toUpperCase()} ${allowed.path} is absent from OpenAPI`,
    );
  }
}

for (const name of Object.keys(PRODUCT_TOOL_ROUTES)) {
  if (!tools.some((tool) => tool.name === name))
    failures.push(`${name}: allowlist entry has no tool`);
}

if (failures.length > 0) {
  process.stderr.write(`${failures.join('\n')}\n`);
  process.exitCode = 1;
}
