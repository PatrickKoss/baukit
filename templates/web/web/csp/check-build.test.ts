import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { findUnhashedInlineScripts } from './check-build.ts';
import { contentSecurityPolicy } from '@baukit/csp';
import { POLICY_FILE, scriptHashSources } from './vite-plugin.ts';

const directories: string[] = [];

async function builtApp(pages: Record<string, string>, policy: string): Promise<string> {
  const outDir = await mkdtemp(join(tmpdir(), 'csp-build-'));
  directories.push(outDir);
  await writeFile(join(outDir, POLICY_FILE), `${policy}\n`);
  for (const [name, html] of Object.entries(pages)) {
    await mkdir(join(outDir, name, '..'), { recursive: true });
    await writeFile(join(outDir, name), html);
  }
  return outDir;
}

afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true })));
});

describe('build check', () => {
  const inline = '<script>window.boot = true;</script>';

  it('fails on an inline script without a matching hash', async () => {
    const policy = contentSecurityPolicy({ styles: 'files', origins: {}, scriptHashes: [] });
    const outDir = await builtApp({ 'index.html': inline, 'nested/page.html': inline }, policy);

    const findings = await findUnhashedInlineScripts(outDir);

    expect(findings).toHaveLength(2);
    expect(findings[0]).toMatch(/^(index|nested\/page)\.html: 'sha256-/u);
  });

  it('passes when the policy hashes every inline script', async () => {
    const policy = contentSecurityPolicy({
      styles: 'files',
      origins: {},
      scriptHashes: scriptHashSources(inline),
    });
    const outDir = await builtApp({ 'index.html': inline }, policy);

    await expect(findUnhashedInlineScripts(outDir)).resolves.toEqual([]);
  });
});
