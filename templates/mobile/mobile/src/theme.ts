import { tokens } from './tokens';

export const theme = {
  color: {
    background: tokens.color.background.primary.light,
    surface: '#f4f6f8',
    text: tokens.color.text.primary.light,
    muted: '#59636e',
    accent: tokens.color.background.accent.light,
    onAccent: tokens.color.text.onAccent.light,
    border: '#d8dde3',
    error: '#b42318',
  },
  space: {
    small: tokens.space.small,
    medium: tokens.space.medium,
    large: 24,
  },
  radius: {
    button: tokens.radius.small,
    card: 12,
  },
} as const;
