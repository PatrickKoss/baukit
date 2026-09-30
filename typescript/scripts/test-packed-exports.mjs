// Packs the package in the working directory and resolves every export with
// CommonJS `require` conditions, the way Jest resolves `@baukit/*` imports.
// `--load <subpath>` also loads that export with `require`, which Node 24 runs as require(esm).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, sep } from 'node:path';
import { parseArgs } from 'node:util';

const { values } = parseArgs({
  options: {
    'esm-only': { type: 'string', multiple: true, default: [] },
    load: { type: 'string', multiple: true, default: [] },
  },
});
const esmOnlySubpaths = new Set(values['esm-only']);
const loadedSubpaths = values['load'];
const packageRoot = process.cwd();
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
const workDirectory = await mkdtemp(join(tmpdir(), 'baukit-packed-exports-'));

function pack(destination) {
  const pnpmCli = process.env['npm_execpath'];
  assert.ok(pnpmCli, 'run this through a pnpm script so npm_execpath is set');
  const isScript = /\.[cm]?js$/.test(pnpmCli);
  const [command, prefix] = isScript ? [process.execPath, [pnpmCli]] : [pnpmCli, []];
  execFileSync(command, [...prefix, 'pack', '--pack-destination', destination], {
    cwd: packageRoot,
    stdio: 'pipe',
  });
}

async function installPacked() {
  pack(workDirectory);
  const archives = (await readdir(workDirectory)).filter((name) => name.endsWith('.tgz'));
  assert.equal(archives.length, 1, 'expected exactly one packed archive');
  const installed = join(workDirectory, 'node_modules', ...manifest.name.split('/'));
  await mkdir(installed, { recursive: true });
  execFileSync('tar', [
    '-xzf',
    join(workDirectory, archives[0]),
    '-C',
    installed,
    '--strip-components=1',
  ]);
  return installed;
}

function specifier(subpath) {
  return subpath === '.' ? manifest.name : `${manifest.name}/${subpath.slice('./'.length)}`;
}

try {
  const installed = await installPacked();
  const consumerRequire = createRequire(join(workDirectory, 'consumer.cjs'));
  const subpaths = Object.keys(manifest.exports);
  for (const subpath of esmOnlySubpaths) {
    assert.ok(subpaths.includes(subpath), `--esm-only ${subpath} is not an export`);
    assert.throws(() => consumerRequire.resolve(specifier(subpath)), {
      code: 'ERR_PACKAGE_PATH_NOT_EXPORTED',
    });
  }
  const requirable = subpaths.filter((subpath) => !esmOnlySubpaths.has(subpath));
  for (const subpath of requirable) {
    const resolved = consumerRequire.resolve(specifier(subpath));
    assert.ok(resolved.startsWith(installed + sep), `${subpath} resolved outside the package`);
    assert.ok(existsSync(resolved), `${subpath} resolves to a file missing from the archive`);
  }
  for (const subpath of loadedSubpaths) {
    assert.ok(requirable.includes(subpath), `--load ${subpath} is not a requirable export`);
    const loaded = consumerRequire(specifier(subpath));
    assert.ok(Object.keys(loaded).length > 0, `${subpath} loaded without exports`);
  }
  console.log(
    `${manifest.name}: ${requirable.length} export(s) resolve under require conditions from the packed archive, ${loadedSubpaths.length} load with require.`,
  );
} finally {
  await rm(workDirectory, { recursive: true, force: true });
}
