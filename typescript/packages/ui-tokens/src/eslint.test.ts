import { RuleTester } from 'eslint';
import { afterAll, describe, expect, it } from 'vitest';

import { configs, noRawColor, rules } from './eslint.js';

RuleTester.afterAll = afterAll;
RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const ruleTester = new RuleTester({
  languageOptions: { ecmaVersion: 2022, sourceType: 'module' },
});

const jsx = new RuleTester({
  languageOptions: {
    ecmaVersion: 2022,
    sourceType: 'module',
    parserOptions: { ecmaFeatures: { jsx: true } },
  },
});

ruleTester.run('no-raw-color', noRawColor, {
  valid: [
    { name: 'token reference', code: 'const style = { color: theme.textPrimary };' },
    { name: 'allowed default keyword', code: 'const style = { backgroundColor: "transparent" };' },
    { name: 'currentColor', code: 'const style = { fill: "currentColor" };' },
    {
      name: 'keyword outside a style context',
      code: 'const route = "red"; const id = "black-list";',
    },
    {
      name: 'non-color string in a style context',
      code: 'const style = { color: theme.get("primary") };',
    },
    {
      name: 'allow-listed literal',
      code: 'const style = { color: "#123456" };',
      options: [{ allowedValues: ['#123456'] }],
    },
    {
      name: 'token definition file',
      code: 'export const palette = { accent: "#0b57d0" };',
      filename: '/repo/src/theme/tokens.ts',
      options: [{ allowedFiles: ['src/theme/**'] }],
    },
    {
      name: 'keyword reporting disabled',
      code: 'const style = { color: "red" };',
      options: [{ reportKeywords: false }],
    },
    { name: 'hash that is not a color', code: 'const anchor = "#section-heading";' },
    { name: 'plain sentence', code: 'const copy = "the red car is fast";' },
  ],
  invalid: [
    {
      name: 'six-digit hex anywhere',
      code: 'const accent = "#0b57d0";',
      errors: [{ messageId: 'rawHex', data: { value: '#0b57d0' } }],
    },
    {
      name: 'three-digit hex',
      code: 'const accent = "#fff";',
      errors: [{ messageId: 'rawHex' }],
    },
    {
      name: 'eight-digit hex with alpha',
      code: 'const overlay = "#0b57d080";',
      errors: [{ messageId: 'rawHex' }],
    },
    {
      name: 'rgb function',
      code: 'const style = { color: "rgb(11, 87, 208)" };',
      errors: [{ messageId: 'rawFunction', data: { value: 'rgb' } }],
    },
    {
      name: 'rgba function',
      code: 'const style = { backgroundColor: "rgba(0, 0, 0, 0.5)" };',
      errors: [{ messageId: 'rawFunction' }],
    },
    {
      name: 'hsl function',
      code: 'const style = { borderColor: "hsl(210 100% 50%)" };',
      errors: [{ messageId: 'rawFunction' }],
    },
    {
      name: 'hsla function',
      code: 'const style = { shadowColor: "hsla(210, 100%, 50%, 0.4)" };',
      errors: [{ messageId: 'rawFunction' }],
    },
    {
      name: 'keyword in a style property',
      code: 'const style = { color: "rebeccapurple" };',
      errors: [{ messageId: 'rawKeyword', data: { value: 'rebeccapurple' } }],
    },
    {
      name: 'keyword nested under a style property',
      code: 'const style = { colors: { danger: "crimson" } };',
      errors: [{ messageId: 'rawKeyword' }],
    },
    {
      name: 'keyword inside a StyleSheet.create call',
      code: 'const styles = StyleSheet.create({ card: { backgroundColor: "white" } });',
      errors: [{ messageId: 'rawKeyword' }],
    },
    {
      name: 'template literal in a css tag',
      code: 'const card = css`color: #0b57d0;`;',
      errors: [{ messageId: 'rawHex' }],
    },
    {
      name: 'template literal with an expression around a hex',
      code: 'const shade = `${base} #ff0000`;',
      errors: [{ messageId: 'rawHex' }],
    },
    {
      name: 'style-named variable holding a keyword',
      code: 'const cardStyle = { borderColor: "silver" };',
      errors: [{ messageId: 'rawKeyword' }],
    },
    {
      name: 'file outside the allow-list',
      code: 'const accent = "#0b57d0";',
      filename: '/repo/src/components/card.ts',
      options: [{ allowedFiles: ['src/theme/**'] }],
      errors: [{ messageId: 'rawHex' }],
    },
    {
      name: 'custom style property name',
      code: 'const props = { chartFill: "teal" };',
      options: [{ additionalStyleProperties: ['chartFill'] }],
      errors: [{ messageId: 'rawKeyword' }],
    },
  ],
});

jsx.run('no-raw-color in JSX', noRawColor, {
  valid: [
    { name: 'token expression', code: 'const el = <View style={{ color: theme.accent }} />;' },
    { name: 'non-style attribute keyword', code: 'const el = <View testID="red" />;' },
  ],
  invalid: [
    {
      name: 'inline style attribute',
      code: 'const el = <View style={{ backgroundColor: "#0b57d0" }} />;',
      errors: [{ messageId: 'rawHex' }],
    },
    {
      name: 'color attribute keyword',
      code: 'const el = <Icon color="tomato" />;',
      errors: [{ messageId: 'rawKeyword' }],
    },
  ],
});

describe('plugin exports', () => {
  it('exposes the rule and a recommended flat config', () => {
    expect(rules['no-raw-color']).toBe(noRawColor);
    expect(configs.recommended.rules).toEqual({ '@baukit/ui-tokens/no-raw-color': 'error' });
    expect(configs.recommended.plugins['@baukit/ui-tokens'].rules).toBe(rules);
  });
});
