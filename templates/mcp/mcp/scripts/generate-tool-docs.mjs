import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { READ_TOOLS } from '../dist/tools/read.js';
import { WRITE_TOOLS } from '../dist/tools/write.js';

const target = process.env['MCP_TOOL_DOCS_PATH']
  ? pathToFileURL(resolve(process.env['MCP_TOOL_DOCS_PATH']))
  : new URL('../docs/tools.md', import.meta.url);
const rendered = renderTools();

if (process.argv.includes('--check')) {
  const committed = await readFile(target, 'utf8');
  if (committed !== rendered) {
    process.stderr.write(
      'docs/tools.md does not match the tool registries. Run pnpm run docs.\n',
    );
    process.exitCode = 1;
  }
} else {
  await writeFile(target, rendered);
}

function renderTools() {
  const sections = [
    '# MCP tools',
    '',
    'This file is generated from the read and write registries. Run `pnpm run docs` after changing either registry.',
    '',
    renderSection('Read tools', READ_TOOLS),
    '',
    renderSection('Write tools', WRITE_TOOLS),
  ];
  return `${sections.join('\n')}\n`;
}

function renderSection(title, tools) {
  const lines = [`## ${title}`, ''];
  if (tools.length === 0) {
    lines.push('None.');
    return lines.join('\n');
  }
  for (const tool of tools) {
    lines.push(`### \`${tool.name}\``, '', tool.description, '');
    lines.push(
      `Route: \`${tool.route.method.toUpperCase()} ${tool.route.path}\``,
      '',
    );
    lines.push(
      `Annotations: read-only=${String(tool.annotations.readOnlyHint)}, destructive=${String(tool.annotations.destructiveHint)}, idempotent=${String(tool.annotations.idempotentHint)}, open-world=${String(tool.annotations.openWorldHint)}`,
      '',
    );
  }
  lines.pop();
  return lines.join('\n');
}
