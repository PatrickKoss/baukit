import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const packageRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const outputDirectory = await mkdtemp(join(tmpdir(), 'baukit-pwa-web-pack-'));

try {
  const pnpmCli = process.env['npm_execpath'];
  assert.ok(pnpmCli, 'npm_execpath is required to test the pnpm package');
  execFileSync(process.execPath, [pnpmCli, 'pack', '--pack-destination', outputDirectory], {
    cwd: packageRoot,
    stdio: 'pipe',
  });

  const archives = (await readdir(outputDirectory)).filter((name) => name.endsWith('.tgz'));
  assert.equal(archives.length, 1);
  const archive = join(outputDirectory, archives[0]);
  const files = execFileSync('tar', ['-tzf', archive], { encoding: 'utf8' })
    .trim()
    .split('\n')
    .sort();

  for (const required of [
    'package/LICENSE',
    'package/README.md',
    'package/dist/index.d.ts',
    'package/dist/index.js',
    'package/dist/worker.js',
    'package/package.json',
  ]) {
    assert.ok(files.includes(required), `packed package is missing ${required}`);
  }
  assert.equal(
    files.some((name) => name.startsWith('package/src/')),
    false,
  );

  const packedManifest = JSON.parse(
    execFileSync('tar', ['-xOzf', archive, 'package/package.json'], { encoding: 'utf8' }),
  );
  assert.equal(packedManifest.exports['./worker'], './dist/worker.js');

  const packedWorker = execFileSync('tar', ['-xOzf', archive, 'package/dist/worker.js'], {
    encoding: 'utf8',
  });
  assert.equal(packedWorker, await readFile(join(packageRoot, 'dist/worker.js'), 'utf8'));
  console.log(`Packed artifact contains ${files.length} files, including dist/worker.js.`);
} finally {
  await rm(outputDirectory, { recursive: true, force: true });
}
