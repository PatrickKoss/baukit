import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { ESLint } from 'eslint';
import { base, baseRules, react, scripts, testFiles } from './index.js';
import { reactWeb } from './react-web.js';

async function lintScript(code) {
  const eslint = new ESLint({ overrideConfigFile: true, overrideConfig: scripts() });
  const [result] = await eslint.lintText(code, { filePath: 'tool.mjs' });
  return result.messages.map((message) => message.ruleId);
}

describe('@baukit/eslint-config', () => {
  it('reports functions with more than four parameters in scripts', async () => {
    const rules = await lintScript(
      'export function f(a, b, c, d, e) { return a + b + c + d + e; }\n',
    );
    assert.deepEqual(rules, ['max-params']);
  });

  it('accepts four parameters and Node globals in scripts', async () => {
    const rules = await lintScript(
      'export function f(a, b, c, d) { return process.argv[a + b + c + d]; }\n',
    );
    assert.deepEqual(rules, []);
  });

  it('rejects a disable directive without a reason and a blanket disable', async () => {
    const bare = await lintScript('// eslint-disable-next-line no-undef\nexport const x = y;\n');
    assert.deepEqual(bare, ['@eslint-community/eslint-comments/require-description']);
    const blanket = await lintScript('/* eslint-disable */\nexport const x = 1;\n');
    assert.ok(blanket.includes('@eslint-community/eslint-comments/no-unlimited-disable'));
  });

  it('accepts a disable directive with a reason and reports an unused one', async () => {
    const reasoned = await lintScript(
      '// eslint-disable-next-line no-undef -- y is injected by the host\nexport const x = y;\n',
    );
    assert.deepEqual(reasoned, []);
    const unused = await lintScript(
      '// eslint-disable-next-line no-undef -- stale\nexport const x = 1;\n',
    );
    assert.deepEqual(unused, [null]);
  });

  it('scopes every base block to the requested files and relaxes test size limits', () => {
    const configs = base({ files: ['src/**/*.ts'] });
    const testBlock = configs.at(-1);
    assert.deepEqual(testBlock.files, testFiles);
    assert.equal(testBlock.rules['max-lines-per-function'], 'off');
    assert.deepEqual(configs.at(-3).rules, baseRules);
    for (const config of configs.slice(0, -1)) assert.deepEqual(config.files, ['src/**/*.ts']);
  });

  it('adds react hooks to react and jsx-a11y to reactWeb', () => {
    const hooks = react().at(-1);
    assert.equal(hooks.rules['react-hooks/rules-of-hooks'], 'error');
    const a11y = reactWeb({ domFiles: ['src/web.tsx'] }).at(-1);
    assert.deepEqual(a11y.files, ['src/web.tsx']);
    assert.ok(Object.keys(a11y.rules).some((rule) => rule.startsWith('jsx-a11y/')));
  });
});
