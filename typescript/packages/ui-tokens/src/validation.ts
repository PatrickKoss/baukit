import type { DesignTokens, ThemeColor } from './schema.js';

export interface ValidationIssue {
  readonly path: string;
  readonly message: string;
}

const TOKEN_NAME = /^[a-z][A-Za-z0-9]*$/u;
const HEX_COLOR = /^#[\dA-Fa-f]{3}(?:[\dA-Fa-f]{3})?$/u;

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function issue(issues: ValidationIssue[], path: string, message: string): void {
  issues.push({ path, message });
}

function validateKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
  path: string,
  issues: ValidationIssue[],
): void {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) {
      issue(issues, `${path}.${key}`, 'is not a recognized field');
    }
  }
  for (const key of allowed) {
    if (!(key in value)) {
      issue(issues, `${path}.${key}`, 'is required');
    }
  }
}

function validateTokenName(name: string, path: string, issues: ValidationIssue[]): void {
  if (!TOKEN_NAME.test(name)) {
    issue(issues, path, 'must start with a lowercase letter and contain only letters or digits');
  }
}

function validateColorGroup(
  value: unknown,
  path: string,
  depth: number,
  issues: ValidationIssue[],
): void {
  if (!isRecord(value)) {
    issue(issues, path, 'must be a color token group');
    return;
  }

  const isLeaf = 'light' in value || 'dark' in value;
  if (isLeaf) {
    validateKeys(value, ['light', 'dark'], path, issues);
    if (depth < 2) {
      issue(
        issues,
        path,
        'must use a semantic group and token name (for example background.primary)',
      );
    }
    for (const theme of ['light', 'dark'] as const) {
      const color = value[theme];
      if (typeof color !== 'string' || !HEX_COLOR.test(color)) {
        issue(issues, `${path}.${theme}`, 'must be a #RGB or #RRGGBB hexadecimal color');
      }
    }
    return;
  }

  const entries = Object.entries(value);
  if (entries.length === 0) {
    issue(issues, path, 'must contain at least one token');
  }
  for (const [name, child] of entries) {
    validateTokenName(name, `${path}.${name}`, issues);
    validateColorGroup(child, `${path}.${name}`, depth + 1, issues);
  }
}

type ScalarKind = 'dimension' | 'positive-dimension' | 'string' | 'weight';

function validScalar(value: unknown, kind: ScalarKind): boolean {
  if (kind === 'string') {
    return typeof value === 'string' && value.length > 0;
  }
  if (kind === 'weight') {
    return typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= 1000;
  }
  if (typeof value === 'string') {
    return value.length > 0;
  }
  return (
    typeof value === 'number' &&
    Number.isFinite(value) &&
    (kind === 'positive-dimension' ? value > 0 : value >= 0)
  );
}

function validateScale(
  value: unknown,
  path: string,
  kind: ScalarKind,
  issues: ValidationIssue[],
): void {
  if (!isRecord(value)) {
    issue(issues, path, 'must be a token scale');
    return;
  }
  const entries = Object.entries(value);
  if (entries.length === 0) {
    issue(issues, path, 'must contain at least one token');
  }
  for (const [name, token] of entries) {
    const tokenPath = `${path}.${name}`;
    validateTokenName(name, tokenPath, issues);
    if (!validScalar(token, kind)) {
      issue(issues, tokenPath, `must be a valid ${kind.replace('-', ' ')}`);
    }
  }
}

function validateTypography(value: unknown, issues: ValidationIssue[]): void {
  if (!isRecord(value)) {
    issue(issues, '$.typography', 'must be an object');
    return;
  }
  validateKeys(value, ['family', 'size', 'weight', 'lineHeight'], '$.typography', issues);
  validateScale(value['family'], '$.typography.family', 'string', issues);
  validateScale(value['size'], '$.typography.size', 'positive-dimension', issues);
  validateScale(value['weight'], '$.typography.weight', 'weight', issues);
  validateScale(value['lineHeight'], '$.typography.lineHeight', 'positive-dimension', issues);
}

function validateMotion(value: unknown, issues: ValidationIssue[]): void {
  if (!isRecord(value)) {
    issue(issues, '$.motion', 'must be an object');
    return;
  }
  validateKeys(value, ['duration', 'easing'], '$.motion', issues);
  validateScale(value['duration'], '$.motion.duration', 'dimension', issues);
  validateScale(value['easing'], '$.motion.easing', 'string', issues);
}

function findColor(value: unknown, path: string): ThemeColor | undefined {
  const segments = path.split('.');
  if (segments.shift() !== 'color') {
    return undefined;
  }
  let current: unknown = value;
  for (const segment of segments) {
    if (!isRecord(current)) {
      return undefined;
    }
    current = current[segment];
  }
  if (
    isRecord(current) &&
    typeof current['light'] === 'string' &&
    typeof current['dark'] === 'string'
  ) {
    return current as unknown as ThemeColor;
  }
  return undefined;
}

function validateContrastPairs(value: unknown, colors: unknown, issues: ValidationIssue[]): void {
  if (!Array.isArray(value)) {
    issue(issues, '$.contrastPairs', 'must be an array');
    return;
  }
  value.forEach((pair: unknown, index: number) => {
    const path = `$.contrastPairs[${String(index)}]`;
    if (!isRecord(pair)) {
      issue(issues, path, 'must be an object');
      return;
    }
    for (const key of Object.keys(pair)) {
      if (!['foreground', 'background', 'largeText'].includes(key)) {
        issue(issues, `${path}.${key}`, 'is not a recognized field');
      }
    }
    for (const role of ['foreground', 'background'] as const) {
      const colorPath = pair[role];
      if (typeof colorPath !== 'string' || findColor(colors, colorPath) === undefined) {
        issue(issues, `${path}.${role}`, 'must reference an existing color token path');
      }
    }
    if ('largeText' in pair && typeof pair['largeText'] !== 'boolean') {
      issue(issues, `${path}.largeText`, 'must be a boolean');
    }
  });
}

/** Returns every structural problem with a path rooted at `$`. */
export function validateTokens(input: unknown): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  if (!isRecord(input)) {
    return [{ path: '$', message: 'must be an object' }];
  }

  validateKeys(
    input,
    ['color', 'typography', 'space', 'radius', 'motion', 'elevation', 'contrastPairs'],
    '$',
    issues,
  );
  validateColorGroup(input['color'], '$.color', 0, issues);
  validateTypography(input['typography'], issues);
  validateScale(input['space'], '$.space', 'dimension', issues);
  validateScale(input['radius'], '$.radius', 'dimension', issues);
  validateMotion(input['motion'], issues);
  validateScale(input['elevation'], '$.elevation', 'dimension', issues);
  validateContrastPairs(input['contrastPairs'], input['color'], issues);
  return issues;
}

export class TokenValidationError extends Error {
  public readonly issues: readonly ValidationIssue[];

  public constructor(issues: readonly ValidationIssue[]) {
    super(issues.map(({ path, message }) => `${path}: ${message}`).join('\n'));
    this.name = 'TokenValidationError';
    this.issues = issues;
  }
}

/** Validates unknown input and returns it with the design-token type. */
export function parseTokens(input: unknown): DesignTokens {
  const issues = validateTokens(input);
  if (issues.length > 0) {
    throw new TokenValidationError(issues);
  }
  return input as DesignTokens;
}
