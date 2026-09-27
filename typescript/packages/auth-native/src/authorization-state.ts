/** Minimum random bytes behind a decorated authorization state. */
export const AUTHORIZATION_STATE_ENTROPY_BYTES = 32;

const STATE_SEGMENT = /^[A-Za-z0-9]+$/;
const HEX_COLOR = /^#([0-9A-Fa-f]{6})$/;
const APPEARANCE_STATE_VERSION = 'ap1';
const APPEARANCE_MODE_CODES = { dark: 'd', light: 'l', system: 's' } as const;

export type AppearanceMode = keyof typeof APPEARANCE_MODE_CODES;

export interface AppearanceState {
  readonly mode: AppearanceMode;
  /** `#RRGGBB`. Pass both colors or neither. */
  readonly primaryColor?: string;
  readonly secondaryColor?: string;
}

/**
 * State segments that tell a login theme the app's appearance:
 * `ap1.<d|l|s>[.<PRIMARY>.<SECONDARY>]`. Throws TypeError on malformed colors.
 */
export function appearanceStateDecoration(appearance: AppearanceState): readonly string[] {
  return [
    APPEARANCE_STATE_VERSION,
    APPEARANCE_MODE_CODES[appearance.mode],
    ...appearanceColorSegments(appearance),
  ];
}

/**
 * Joins the decoration and a hex nonce with dots. Throws when the entropy is
 * shorter than {@link AUTHORIZATION_STATE_ENTROPY_BYTES} or a segment is not
 * ASCII letters and digits.
 */
export function decoratedAuthorizationState(
  decoration: readonly string[],
  entropy: Uint8Array,
): string {
  if (entropy.length < AUTHORIZATION_STATE_ENTROPY_BYTES) {
    throw new RangeError(
      `Authorization state needs at least ${String(AUTHORIZATION_STATE_ENTROPY_BYTES)} random bytes.`,
    );
  }
  if (decoration.length === 0 || !decoration.every((segment) => STATE_SEGMENT.test(segment))) {
    throw new TypeError('State decoration segments must be non-empty ASCII letters or digits.');
  }
  return [...decoration, hexNonce(entropy)].join('.');
}

function appearanceColorSegments(appearance: AppearanceState): readonly string[] {
  const { primaryColor, secondaryColor } = appearance;
  if (primaryColor === undefined && secondaryColor === undefined) return [];
  if (primaryColor === undefined || secondaryColor === undefined) {
    throw new TypeError('Appearance state needs both colors or neither.');
  }
  return [colorSegment(primaryColor), colorSegment(secondaryColor)];
}

function colorSegment(color: string): string {
  const match = HEX_COLOR.exec(color);
  if (match?.[1] === undefined) {
    throw new TypeError('Appearance colors must be #RRGGBB.');
  }
  return match[1].toUpperCase();
}

function hexNonce(entropy: Uint8Array): string {
  return Array.from(entropy, (byte) => byte.toString(16).padStart(2, '0')).join('');
}
