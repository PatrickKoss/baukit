import { writeFile } from 'node:fs/promises';

import { exampleTokens, toCssVariables } from '@baukit/ui-tokens';

const output = new URL('../src/tokens.css', import.meta.url);
await writeFile(output, toCssVariables(exampleTokens), 'utf8');
