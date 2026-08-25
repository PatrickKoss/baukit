import type { DesignTokens } from './schema.js';

/** A deliberately small fixture that demonstrates the pipeline, not a design system. */
export const exampleTokens = {
  color: {
    background: {
      primary: { light: '#ffffff', dark: '#111111' },
      accent: { light: '#005fcc', dark: '#66aaff' },
    },
    text: {
      primary: { light: '#111111', dark: '#ffffff' },
      onAccent: { light: '#ffffff', dark: '#111111' },
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
