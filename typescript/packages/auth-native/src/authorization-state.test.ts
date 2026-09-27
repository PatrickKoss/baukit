import { describe, expect, it } from 'vitest';

import {
  AUTHORIZATION_STATE_ENTROPY_BYTES,
  appearanceStateDecoration,
  decoratedAuthorizationState,
} from './index.js';

const entropy = Uint8Array.from({ length: AUTHORIZATION_STATE_ENTROPY_BYTES }, (_, index) => index);
const nonce = Array.from(entropy, (byte) => byte.toString(16).padStart(2, '0')).join('');

describe('appearanceStateDecoration', () => {
  it('encodes the mode alone', () => {
    expect(appearanceStateDecoration({ mode: 'dark' })).toEqual(['ap1', 'd']);
    expect(appearanceStateDecoration({ mode: 'light' })).toEqual(['ap1', 'l']);
    expect(appearanceStateDecoration({ mode: 'system' })).toEqual(['ap1', 's']);
  });

  it('encodes both colors as uppercase hex without the hash', () => {
    expect(
      appearanceStateDecoration({
        mode: 'light',
        primaryColor: '#0a7cff',
        secondaryColor: '#FF5500',
      }),
    ).toEqual(['ap1', 'l', '0A7CFF', 'FF5500']);
  });

  it('rejects a single color or a malformed color', () => {
    expect(() => appearanceStateDecoration({ mode: 'dark', primaryColor: '#000000' })).toThrow(
      TypeError,
    );
    expect(() =>
      appearanceStateDecoration({ mode: 'dark', primaryColor: 'red', secondaryColor: '#000000' }),
    ).toThrow('#RRGGBB');
  });
});

describe('decoratedAuthorizationState', () => {
  it('appends a hex nonce to the decoration', () => {
    expect(decoratedAuthorizationState(['ap1', 'd'], entropy)).toBe(`ap1.d.${nonce}`);
  });

  it('refuses short entropy', () => {
    expect(() => decoratedAuthorizationState(['ap1'], entropy.subarray(1))).toThrow(RangeError);
  });

  it('refuses empty decorations and segments outside letters and digits', () => {
    expect(() => decoratedAuthorizationState([], entropy)).toThrow(TypeError);
    expect(() => decoratedAuthorizationState([''], entropy)).toThrow(TypeError);
    expect(() => decoratedAuthorizationState(['a.b'], entropy)).toThrow(TypeError);
    expect(() => decoratedAuthorizationState(['a&b=c'], entropy)).toThrow(TypeError);
  });
});
