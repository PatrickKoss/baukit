import assert from 'node:assert/strict';
import { readFile, readdir, stat } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

async function checkMap(map, mapPath, packageRoot) {
  assert.equal(map.version, 3, `${mapPath}: unsupported source map version`);
  assert.ok(Array.isArray(map.sources), `${mapPath}: source map must list sources`);
  for (const [index, source] of map.sources.entries()) {
    assert.equal(typeof source, 'string', `${mapPath}: source must be a path`);
    if (typeof map.sourcesContent?.[index] === 'string') continue;
    const sourcePath = resolve(dirname(mapPath), map.sourceRoot ?? '', source);
    const packagePath = relative(packageRoot, sourcePath);
    assert.ok(
      packagePath !== '..' && !packagePath.startsWith(`..${sep}`) && !isAbsolute(packagePath),
      `${mapPath}: source ${source} is outside the packed package`,
    );
    let published;
    try {
      published = await stat(sourcePath);
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      assert.fail(`${mapPath}: source ${source} is missing from the packed package`);
    }
    assert.ok(published.isFile(), `${mapPath}: source ${source} is not a published file`);
  }
}

export async function checkPackedSourceMaps(packageRoot) {
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        await visit(path);
      } else if (entry.isFile() && entry.name.endsWith('.map')) {
        await checkMap(JSON.parse(await readFile(path, 'utf8')), path, packageRoot);
      }
    }
  }
  await visit(packageRoot);
}
