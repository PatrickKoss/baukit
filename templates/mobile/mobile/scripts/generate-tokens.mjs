import { writeFile } from 'node:fs/promises';

import { exampleTokens, toReactNative } from '@baukit/ui-tokens';

const output = new URL('../src/tokens.ts', import.meta.url);
await writeFile(output, toReactNative(exampleTokens), 'utf8');
