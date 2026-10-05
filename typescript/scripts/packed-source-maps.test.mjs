import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { checkPackedSourceMaps } from './packed-source-maps.mjs';

async function fixture(t, map, file = 'index.js.map') {
  const root = await mkdtemp(join(tmpdir(), 'baukit-map-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'dist/cjs'), { recursive: true });
  await writeFile(join(root, 'dist', file), JSON.stringify({ version: 3, ...map }));
  return root;
}

test('external JavaScript and declaration maps resolve published sources', async (t) => {
  const root = await fixture(t, { sources: ['../src/index.ts'] });
  await mkdir(join(root, 'src'));
  await writeFile(join(root, 'src/index.ts'), 'export const value = 1;');
  await writeFile(
    join(root, 'dist/cjs/index.d.ts.map'),
    JSON.stringify({ version: 3, sourceRoot: '../../', sources: ['src/index.ts'] }),
  );
  await checkPackedSourceMaps(root);
});

for (const file of ['index.js.map', 'index.d.ts.map']) {
  test(`${file} rejects an unpublished source`, async (t) => {
    const root = await fixture(t, { sources: ['../src/index.ts'] }, file);
    await assert.rejects(
      checkPackedSourceMaps(root),
      /source .* is missing from the packed package/,
    );
  });
}

test('inline source content needs no published source file', async (t) => {
  const root = await fixture(t, { sources: ['../src/index.ts'], sourcesContent: [''] });
  await checkPackedSourceMaps(root);
});

test('null source content still requires a published source', async (t) => {
  const root = await fixture(t, { sources: ['../src/index.ts'], sourcesContent: [null] });
  await assert.rejects(checkPackedSourceMaps(root), /missing from the packed package/);
});

test('a file outside the archive cannot satisfy a map', async (t) => {
  const root = await fixture(t, { sources: ['../../outside.ts'] });
  await assert.rejects(checkPackedSourceMaps(root), /outside the packed package/);
});
