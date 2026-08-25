// eslint-disable-next-line @typescript-eslint/triple-slash-reference
/// <reference path="./raw.d.ts" />

import { describe, expect, it } from 'vitest';

import fixtureJson from '../../../../fixtures/product-experience/optional-wire-values.json?raw';
import { decodeOptionalWireValue, encodeOptionalWireValue } from './wire.js';

type FixtureScalar = boolean | string;

interface OptionalWireFixture {
  readonly name: string;
  readonly key: string;
  readonly wire: 'absent' | Record<string, FixtureScalar | null>;
  readonly expectedState: 'absent' | 'null' | 'value';
  readonly expectedValue?: FixtureScalar;
  readonly roundTrip: 'absent' | Record<string, FixtureScalar | null>;
}

const fixtures = JSON.parse(fixtureJson) as OptionalWireFixture[];

describe('optional wire value fixtures', () => {
  it('contains the product-experience source cases', () => {
    expect(fixtures.map(({ name }) => name)).toEqual(
      expect.arrayContaining([
        'language preference from old peer',
        'theme mode value',
        'game-layer boolean value',
        'custom color from old peer',
        'custom color explicitly cleared',
        'custom color selected',
      ]),
    );
  });

  for (const fixture of fixtures) {
    it(`decodes and encodes ${fixture.name}`, () => {
      const payload = fixture.wire === 'absent' ? {} : fixture.wire;
      const decoded = decodeOptionalWireValue(payload, fixture.key, (value) => {
        if (typeof value !== 'string' && typeof value !== 'boolean') {
          throw new TypeError('Fixture value must be a string or boolean');
        }
        return value;
      });

      expect(decoded.state).toBe(fixture.expectedState);
      if (decoded.state === 'value') {
        expect(decoded.value).toBe(fixture.expectedValue);
      }

      const encoded = encodeOptionalWireValue(fixture.key, decoded);
      const jsonRoundTrip = JSON.parse(JSON.stringify(encoded)) as Record<
        string,
        FixtureScalar | null
      >;
      const representedRoundTrip = Object.hasOwn(jsonRoundTrip, fixture.key)
        ? jsonRoundTrip
        : 'absent';
      expect(representedRoundTrip).toEqual(fixture.roundTrip);
    });
  }
});

describe('optional wire value codec', () => {
  it('does not infer presence from an undefined property value', () => {
    expect(decodeOptionalWireValue({ color: undefined }, 'color', (value) => value)).toEqual({
      state: 'value',
      value: undefined,
    });
  });

  it('rejects payloads that cannot contain a field', () => {
    expect(() => decodeOptionalWireValue(null, 'color', String)).toThrow(TypeError);
    expect(() => decodeOptionalWireValue([], 'color', String)).toThrow(TypeError);
  });
});
