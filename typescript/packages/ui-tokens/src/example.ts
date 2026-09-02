import type { DesignTokens } from './schema.js';
import { checkSemanticContrastMatrix, DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS } from './contrast.js';

/** A deliberately small fixture that demonstrates the pipeline, not a design system. */
export const exampleTokens = {
  color: {
    background: {
      primary: { light: '#ffffff', dark: '#111111' },
      elevated: { light: '#f5f5f5', dark: '#222222' },
      accent: { light: '#005fcc', dark: '#66aaff' },
    },
    text: {
      primary: { light: '#111111', dark: '#ffffff' },
      muted: { light: '#595959', dark: '#b3b3b3' },
      onAccent: { light: '#ffffff', dark: '#111111' },
    },
    border: {
      primary: { light: '#767676', dark: '#888888' },
    },
    focus: {
      ring: { light: '#005fcc', dark: '#66aaff' },
    },
    action: {
      primary: { light: '#005fcc', dark: '#66aaff' },
      onPrimary: { light: '#ffffff', dark: '#111111' },
      secondary: { light: '#5f3dc4', dark: '#b197fc' },
      onSecondary: { light: '#ffffff', dark: '#111111' },
    },
    status: {
      success: { light: '#137333', dark: '#65d48b' },
      onSuccess: { light: '#ffffff', dark: '#111111' },
      warning: { light: '#8a4b00', dark: '#ffd166' },
      onWarning: { light: '#ffffff', dark: '#111111' },
      danger: { light: '#b3261e', dark: '#ff8a80' },
      onDanger: { light: '#ffffff', dark: '#111111' },
    },
  },
  typography: {
    family: { body: 'Inter, sans-serif' },
    size: { body: '1rem', title: '2rem' },
    weight: { regular: 400, bold: 700 },
    lineHeight: { body: 1.5, title: 1.2 },
  },
  space: { small: 8, medium: 16 },
  radius: { small: 4, pill: 999 },
  motion: {
    duration: { fast: '120ms', normal: '240ms' },
    easing: { standard: 'cubic-bezier(0.2, 0, 0, 1)' },
  },
  elevation: { raised: '0 2px 8px rgb(0 0 0 / 0.2)' },
  contrastPairs: [
    { foreground: 'color.text.primary', background: 'color.background.primary' },
    { foreground: 'color.text.onAccent', background: 'color.background.accent' },
  ],
} as const satisfies DesignTokens;

/** Example result for the package's default semantic contrast requirements. */
export const exampleContrastViolations = checkSemanticContrastMatrix(
  exampleTokens,
  DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS,
);
