import { readdir, readFile } from 'node:fs/promises';
import { join, relative } from 'node:path';

import { unhashedInlineScripts } from './policy.ts';
import { POLICY_FILE, sha256Base64 } from './vite-plugin.ts';

async function htmlFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { recursive: true, withFileTypes: true });
  return entries
    .filter((entry) => entry.isFile() && entry.name.endsWith('.html'))
    .map((entry) => join(entry.parentPath, entry.name));
}

/** Lists every built HTML file with an inline script that `csp.txt` does not hash. */
export async function findUnhashedInlineScripts(outDir: string): Promise<string[]> {
  const policy = await readFile(join(outDir, POLICY_FILE), 'utf8');
  const findings: string[] = [];
  for (const file of await htmlFiles(outDir)) {
    const missing = unhashedInlineScripts(await readFile(file, 'utf8'), policy, sha256Base64);
    findings.push(...missing.map((source) => `${relative(outDir, file)}: ${source}`));
  }
  return findings;
}

if (import.meta.main) {
  const outDir = process.argv[2] ?? 'dist';
  const findings = await findUnhashedInlineScripts(outDir);
  if (findings.length > 0) {
    console.error(`Inline scripts without a matching ${POLICY_FILE} hash:\n${findings.join('\n')}`);
    process.exitCode = 1;
  } else {
    console.log(`Every inline script in ${outDir} has a ${POLICY_FILE} hash.`);
  }
}
