const STATE_SEGMENT = /^[A-Za-z0-9]+$/u;
const HEX_COLOR = /^#([0-9A-Fa-f]{6})$/u;
const APPEARANCE_STATE_VERSION = 'ap1';
const APPEARANCE_MODE_CODES = { dark: 'd', light: 'l', system: 's' } as const;
const STATE_ENTROPY_BYTES = 32;
const HEX_RADIX = 16;
const HEX_DIGITS_PER_BYTE = 2;

export type AppearanceMode = keyof typeof APPEARANCE_MODE_CODES;

export interface AppearanceState {
  readonly mode: AppearanceMode;
  /** `#RRGGBB`. Pass both colors or neither. */
  readonly primaryColor?: string;
  readonly secondaryColor?: string;
}

/** `login` is the default entry. `register` asks the provider for its registration page. */
export type LoginEntry = 'login' | 'register';

export interface LoginOptions {
  /**
   * Segments written ahead of the random state nonce, for example
   * `appearanceStateDecoration({ mode: 'dark' })`. Each segment must be ASCII letters and digits.
   */
  readonly stateDecoration?: readonly string[];
  /** Sends `prompt=create` for `register`. A pending fresh-login requirement does not override it. */
  readonly entry?: LoginEntry;
}

/** State segments for a login theme: `ap1.<d|l|s>[.<PRIMARY>.<SECONDARY>]`. */
export function appearanceStateDecoration(appearance: AppearanceState): readonly string[] {
  return [
    APPEARANCE_STATE_VERSION,
    APPEARANCE_MODE_CODES[appearance.mode],
    ...appearanceColorSegments(appearance),
  ];
}

/** Joins the decoration and a hex nonce with dots. */
export function decoratedAuthorizationState(
  decoration: readonly string[],
  entropy: Uint8Array,
): string {
  if (entropy.length < STATE_ENTROPY_BYTES) {
    throw new RangeError(
      `Authorization state needs at least ${String(STATE_ENTROPY_BYTES)} random bytes.`,
    );
  }
  if (decoration.length === 0 || !decoration.every((segment) => STATE_SEGMENT.test(segment))) {
    throw new TypeError('State decoration segments must be non-empty ASCII letters or digits.');
  }
  const nonce = Array.from(entropy, (byte) =>
    byte.toString(HEX_RADIX).padStart(HEX_DIGITS_PER_BYTE, '0'),
  ).join('');
  return [...decoration, nonce].join('.');
}

export function authorizationPrompt(
  entry: LoginEntry | undefined,
  freshLoginRequired: boolean,
): 'login' | 'create' | undefined {
  if (entry === 'register') return 'create';
  return freshLoginRequired ? 'login' : undefined;
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
