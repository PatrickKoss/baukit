import type { Rule } from 'eslint';
import type { Node } from 'estree';

import { findRawColor, isCssColorKeyword, type RawColorKind } from './color-literals.js';

/** Options accepted by the `no-raw-color` rule. */
export interface NoRawColorOptions {
  /**
   * Glob-like file patterns whose contents may declare raw colors. A token
   * definition file belongs here. `*` matches within a path segment, `**`
   * across segments.
   */
  readonly allowedFiles?: readonly string[];
  /** Exact literal values that stay allowed everywhere, such as `transparent`. */
  readonly allowedValues?: readonly string[];
  /** Extra property or attribute names treated as style-like. */
  readonly additionalStyleProperties?: readonly string[];
  /** Report CSS color keywords such as `red`. Defaults to `true`. */
  readonly reportKeywords?: boolean;
}

const DEFAULT_ALLOWED_VALUES = ['transparent', 'currentColor', 'inherit', 'none'];

/**
 * Property and attribute names whose values are styling input. Keyword matching
 * is limited to these so that a route name or a test id is never reported.
 */
const STYLE_PROPERTY_NAMES = new Set([
  'backgroundColor',
  'background',
  'borderColor',
  'borderBottomColor',
  'borderLeftColor',
  'borderRightColor',
  'borderTopColor',
  'borderStartColor',
  'borderEndColor',
  'color',
  'fill',
  'stroke',
  'shadowColor',
  'textShadowColor',
  'textDecorationColor',
  'tintColor',
  'placeholderTextColor',
  'selectionColor',
  'underlayColor',
  'outlineColor',
  'caretColor',
  'colors',
  'style',
  'styles',
  'theme',
  'palette',
  'gradient',
  'boxShadow',
  'textShadow',
  'borderTop',
  'borderBottom',
  'borderLeft',
  'borderRight',
  'border',
  'outline',
]);

/** Tagged-template tags whose contents are CSS. */
const CSS_TEMPLATE_TAGS = new Set([
  'css',
  'styled',
  'createGlobalStyle',
  'keyframes',
  'injectGlobal',
]);

function globToRegExp(pattern: string): RegExp {
  let source = '';
  for (let index = 0; index < pattern.length; index += 1) {
    const character = pattern[index];
    if (character === '*') {
      if (pattern[index + 1] === '*') {
        source += '.*';
        index += 1;
      } else {
        source += '[^/]*';
      }
      continue;
    }
    if (character === '?') {
      source += '[^/]';
      continue;
    }
    source += character?.replace(/[.+^${}()|[\]\\]/g, '\\$&') ?? '';
  }
  return new RegExp(`(?:^|/)(?:${source})$`);
}

function isAllowedFile(filename: string, patterns: readonly string[]): boolean {
  const normalized = filename.replaceAll('\\', '/');
  return patterns.some((pattern) => globToRegExp(pattern).test(normalized));
}

interface EstreeParent {
  readonly parent?: (Node & EstreeParent) | null;
}

type WithParent = Node & EstreeParent;

function propertyKeyName(node: WithParent): string | undefined {
  const parent = node.parent;
  if (parent === undefined || parent === null) return undefined;
  if (parent.type === 'Property' && parent.value === (node as Node)) {
    const key = parent.key as WithParent;
    if (key.type === 'Identifier') return key.name;
    if (key.type === 'Literal' && typeof key.value === 'string') return key.value;
  }
  return undefined;
}

/**
 * Walks up from a literal to decide whether it lands in a styling position: a
 * style-named property or JSX attribute, a CSS tagged template, or a nested
 * object or array under one of those.
 */
function isStyleContext(node: WithParent, styleNames: ReadonlySet<string>): boolean {
  let current: WithParent | null | undefined = node;
  let child: WithParent | undefined;

  while (current !== undefined && current !== null) {
    if (current.type === 'TaggedTemplateExpression') {
      const tag = current.tag as WithParent;
      if (tag.type === 'Identifier' && CSS_TEMPLATE_TAGS.has(tag.name)) return true;
      if (
        tag.type === 'MemberExpression' &&
        tag.object.type === 'Identifier' &&
        CSS_TEMPLATE_TAGS.has(tag.object.name)
      ) {
        return true;
      }
    }

    if (current.type === 'Property' && child !== undefined && current.value === (child as Node)) {
      const name = propertyKeyName(child);
      if (name !== undefined && styleNames.has(name)) return true;
    }

    const jsxName = jsxAttributeName(current);
    if (jsxName !== undefined && styleNames.has(jsxName)) return true;

    if (current.type === 'VariableDeclarator') {
      const id = current.id as WithParent;
      if (id.type === 'Identifier' && looksLikeStyleName(id.name)) return true;
    }

    if (isStyleSheetCall(current)) return true;

    child = current;
    current = current.parent;
  }
  return false;
}

interface JsxAttributeLike {
  readonly type: string;
  readonly name?: { readonly name?: string };
}

function jsxAttributeName(current: WithParent): string | undefined {
  const candidate = current as unknown as JsxAttributeLike;
  return candidate.type === 'JSXAttribute' ? candidate.name?.name : undefined;
}

function looksLikeStyleName(name: string): boolean {
  return /(^|[a-z])(style|styles|theme|palette|colors?)$/i.test(name);
}

function isStyleSheetCall(node: WithParent): boolean {
  if (node.type !== 'CallExpression') return false;
  const callee = node.callee as WithParent;
  return (
    callee.type === 'MemberExpression' &&
    callee.object.type === 'Identifier' &&
    callee.object.name === 'StyleSheet' &&
    callee.property.type === 'Identifier' &&
    callee.property.name === 'create'
  );
}

const MESSAGE_IDS: Record<RawColorKind, 'rawHex' | 'rawFunction' | 'rawKeyword'> = {
  hex: 'rawHex',
  function: 'rawFunction',
  keyword: 'rawKeyword',
};

/**
 * Reports CSS colors written directly in application code. Colors belong in the
 * token definition, which the `allowedFiles` option exempts.
 */
export const noRawColor: Rule.RuleModule = {
  meta: {
    type: 'problem',
    docs: {
      description: 'Disallow raw CSS colors outside the design-token definition',
    },
    schema: [
      {
        type: 'object',
        properties: {
          allowedFiles: { type: 'array', items: { type: 'string' } },
          allowedValues: { type: 'array', items: { type: 'string' } },
          additionalStyleProperties: { type: 'array', items: { type: 'string' } },
          reportKeywords: { type: 'boolean' },
        },
        additionalProperties: false,
      },
    ],
    messages: {
      rawHex: 'Raw hex color "{{value}}". Use a design token instead.',
      rawFunction: 'Raw {{value}}() color. Use a design token instead.',
      rawKeyword: 'Raw CSS color keyword "{{value}}". Use a design token instead.',
    },
  },

  create(context: Rule.RuleContext): Rule.RuleListener {
    const options = (context.options[0] ?? {}) as NoRawColorOptions;
    const allowedFiles = options.allowedFiles ?? [];
    const allowedValues = new Set(
      [...DEFAULT_ALLOWED_VALUES, ...(options.allowedValues ?? [])].map((value) =>
        value.toLowerCase(),
      ),
    );
    const styleNames = new Set([
      ...STYLE_PROPERTY_NAMES,
      ...(options.additionalStyleProperties ?? []),
    ]);
    const reportKeywords = options.reportKeywords ?? true;

    if (isAllowedFile(context.filename, allowedFiles)) return {};

    const check = (node: WithParent, value: string): void => {
      if (allowedValues.has(value.trim().toLowerCase())) return;

      const styled = isStyleContext(node, styleNames);
      const keywordsHere = reportKeywords && styled;
      const match = findRawColor(value, keywordsHere);
      if (match === undefined) return;
      if (match.kind === 'keyword' && !isCssColorKeyword(value) && !styled) return;

      context.report({
        node,
        messageId: MESSAGE_IDS[match.kind],
        data: { value: match.text },
      });
    };

    return {
      Literal(node) {
        if (typeof node.value === 'string') check(node, node.value);
      },
      TemplateLiteral(node) {
        const raw = node.quasis.map((quasi) => quasi.value.cooked ?? quasi.value.raw).join(' ');
        check(node, raw);
      },
    };
  },
};

/** Rules exported by this plugin, keyed by rule name. */
export const rules = { 'no-raw-color': noRawColor };

const plugin = { rules };

/**
 * Ready-made flat config. Spread it into a product's `eslint.config.js` and pass
 * `allowedFiles` so the token definition may keep its color literals.
 */
export const configs = {
  recommended: {
    plugins: { '@baukit/ui-tokens': plugin },
    rules: { '@baukit/ui-tokens/no-raw-color': 'error' },
  },
} as const;

export default plugin;
