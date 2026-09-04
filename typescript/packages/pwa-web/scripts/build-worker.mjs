import { mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { build } from 'rolldown';

const packageRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const outputDirectory = join(packageRoot, 'dist');
await mkdir(outputDirectory, { recursive: true });

await build({
  input: join(packageRoot, 'src/worker-entry.ts'),
  output: {
    file: join(outputDirectory, 'worker.js'),
    format: 'iife',
    name: 'globalThis.BaukitPwa',
    sourcemap: false,
  },
});
