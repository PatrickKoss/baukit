import { describe, expect, it } from 'vitest';

import {
  blendColors,
  checkSemanticContrastMatrix,
  chooseReadableForeground,
  contrastRatio,
  DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS,
  exampleContrastViolations,
  exampleTokens,
  hexToRgb,
  normalizeHexColor,
  rgbToHex,
  type DesignTokens,
} from './index.js';

describe('normalizeHexColor', () => {
  it.each([
    ['#000', '#000000'],
    ['fff', '#ffffff'],
    ['#AbC', '#aabbcc'],
    ['12EfA0', '#12efa0'],
    ['#ABCDEF', '#abcdef'],
  ])('normalizes %s to %s', (input, expected) => {
    expect(normalizeHexColor(input)).toBe(expected);
  });

  it.each(['', '#12', '#1234', '#12345', '#1234567', 'red', ' #fff', '#ggg'])(
    'rejects unsupported input %j',
    (input) => {
      expect(() => normalizeHexColor(input)).toThrow('expected RGB or RRGGBB with an optional #');
    },
  );
});

describe('hex and RGB conversion', () => {
  it('converts both directions at channel boundaries', () => {
    expect(hexToRgb('#00ff7f')).toEqual({ r: 0, g: 255, b: 127 });
    expect(rgbToHex({ r: 0, g: 255, b: 127 })).toBe('#00ff7f');
  });

  it('uses the same hexadecimal validation when converting to RGB', () => {
    expect(() => hexToRgb('#abcd')).toThrow('Unsupported hexadecimal color');
  });

  it.each([
    [{ r: -1, g: 0, b: 0 }, 'r'],
    [{ r: 0, g: 256, b: 0 }, 'g'],
    [{ r: 0, g: 0.5, b: 0 }, 'g'],
    [{ r: 0, g: 0, b: Number.NaN }, 'b'],
    [{ r: 0, g: 0, b: Number.POSITIVE_INFINITY }, 'b'],
  ] as const)('rejects invalid RGB channels', (color, channel) => {
    expect(() => rgbToHex(color)).toThrow(`RGB channel ${channel}`);
  });
});

describe('blendColors', () => {
  it('blends opaque colors and rounds channels', () => {
    expect(blendColors('#ffffff', '#000000', 0.5)).toBe('#808080');
  });

  it('clamps ratios outside the unit interval', () => {
    expect(blendColors('#fff', '#000', -0.1)).toBe('#000000');
    expect(blendColors('#fff', '#000', 1.1)).toBe('#ffffff');
    expect(blendColors('#fff', '#000', 0)).toBe('#000000');
    expect(blendColors('#fff', '#000', 1)).toBe('#ffffff');
  });

  it.each([Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])(
    'rejects a non-finite ratio %s',
    (ratio) => {
      expect(() => blendColors('#fff', '#000', ratio)).toThrow('Blend ratio must be finite');
    },
  );

  it('rejects invalid colors', () => {
    expect(() => blendColors('white', '#000', 0.5)).toThrow('Unsupported hexadecimal color');
  });
});

describe('chooseReadableForeground', () => {
  it('returns the first candidate that meets the requested ratio', () => {
    const result = chooseReadableForeground('#ffffff', ['#777777', '#000000'], 4.5);

    expect(result.foreground).toBe('#000000');
    expect(result.ratio).toBe(21);
    expect(result.meetsThreshold).toBe(true);
  });

  it('normalizes and returns the first passing candidate', () => {
    const result = chooseReadableForeground('FFFFFF', ['#000', '#111111'], 4.5);

    expect(result.foreground).toBe('#000000');
    expect(result.meetsThreshold).toBe(true);
  });

  it('returns the best candidate and achieved ratio when none meets the threshold', () => {
    const result = chooseReadableForeground('#777777', ['#888888', '#999999'], 7);

    expect(result.foreground).toBe('#999999');
    expect(result.ratio).toBeCloseTo(1.5718, 4);
    expect(result.meetsThreshold).toBe(false);
  });

  it('accepts the minimum and maximum possible contrast thresholds', () => {
    expect(chooseReadableForeground('#fff', ['#fff'], 1).meetsThreshold).toBe(true);
    expect(chooseReadableForeground('#fff', ['#000'], 21).meetsThreshold).toBe(true);
  });

  it('rejects an empty candidate list', () => {
    expect(() => chooseReadableForeground('#fff', [], 4.5)).toThrow(
      'must contain at least one color',
    );
  });

  it.each([0.99, 21.01, Number.NaN, Number.POSITIVE_INFINITY])(
    'rejects an invalid minimum ratio %s',
    (minimumRatio) => {
      expect(() => chooseReadableForeground('#fff', ['#000'], minimumRatio)).toThrow(
        'Minimum contrast ratio must be finite and between 1 and 21',
      );
    },
  );

  it('rejects invalid background and candidate colors', () => {
    expect(() => chooseReadableForeground('white', ['#000'], 4.5)).toThrow(
      'Unsupported hexadecimal color',
    );
    expect(() => chooseReadableForeground('#fff', ['black'], 4.5)).toThrow(
      'Unsupported hexadecimal color',
    );
    expect(() => chooseReadableForeground('#fff', ['#000', 'black'], 4.5)).toThrow(
      'Unsupported hexadecimal color',
    );
  });
});

describe('checkSemanticContrastMatrix', () => {
  it('passes the documented default matrix for the example tokens', () => {
    expect(DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS).toHaveLength(13);
    expect(exampleContrastViolations).toEqual([]);
  });

  it('reports a failing pair in exactly one theme', () => {
    const tokens: DesignTokens = {
      ...exampleTokens,
      color: {
        ...exampleTokens.color,
        text: {
          ...exampleTokens.color.text,
          primary: { light: '#777777', dark: '#ffffff' },
        },
      },
    };
    const requirement = {
      foregroundRole: 'color.text.primary',
      backgroundRole: 'color.background.primary',
      minimumRatio: 4.5,
    } as const;

    const violations = checkSemanticContrastMatrix(tokens, [requirement]);

    expect(violations).toHaveLength(1);
    expect(violations[0]).toMatchObject({
      ...requirement,
      theme: 'light',
    });
    expect(violations[0]?.achievedRatio).toBeCloseTo(contrastRatio('#777', '#fff'), 10);
  });

  it('reports every failing requirement in both themes', () => {
    const requirements = [
      {
        foregroundRole: 'color.text.primary',
        backgroundRole: 'color.background.primary',
        minimumRatio: 21,
      },
      {
        foregroundRole: 'color.text.muted',
        backgroundRole: 'color.background.primary',
        minimumRatio: 21,
      },
    ] as const;

    expect(checkSemanticContrastMatrix(exampleTokens, requirements)).toHaveLength(4);
  });

  it('accepts an empty matrix and the minimum possible ratio', () => {
    expect(checkSemanticContrastMatrix(exampleTokens, [])).toEqual([]);
    expect(
      checkSemanticContrastMatrix(exampleTokens, [
        {
          foregroundRole: 'color.text.primary',
          backgroundRole: 'color.background.primary',
          minimumRatio: 1,
        },
      ]),
    ).toEqual([]);
  });

  it('rejects missing role paths and invalid minimum ratios', () => {
    expect(() =>
      checkSemanticContrastMatrix(exampleTokens, [
        {
          foregroundRole: 'color.text.missing',
          backgroundRole: 'color.background.primary',
          minimumRatio: 4.5,
        },
      ]),
    ).toThrow('does not reference a color token');
    expect(() =>
      checkSemanticContrastMatrix(exampleTokens, [
        {
          foregroundRole: 'text.primary',
          backgroundRole: 'color.background.primary',
          minimumRatio: 4.5,
        },
      ]),
    ).toThrow('must start with "color."');
    for (const minimumRatio of [0.99, 21.01, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(() =>
        checkSemanticContrastMatrix(exampleTokens, [
          {
            foregroundRole: 'color.text.primary',
            backgroundRole: 'color.background.primary',
            minimumRatio,
          },
        ]),
      ).toThrow('Minimum contrast ratio must be finite and between 1 and 21');
    }
  });
});
