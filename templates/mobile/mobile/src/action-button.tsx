import { Pressable, StyleSheet, Text } from 'react-native';

import { useTheme, type AppTheme } from './theme';

export interface ActionButtonProps {
  readonly disabled?: boolean;
  readonly label: string;
  readonly onPress: () => void;
  readonly secondary?: boolean;
}

export function ActionButton({
  disabled = false,
  label,
  onPress,
  secondary = false,
}: ActionButtonProps) {
  const { theme } = useTheme();
  const styles = createStyles(theme);
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled}
      onPress={onPress}
      style={[
        styles.button,
        secondary ? styles.buttonSecondary : undefined,
        disabled ? styles.buttonDisabled : undefined,
      ]}
    >
      <Text
        style={[
          styles.buttonText,
          secondary ? styles.buttonTextSecondary : undefined,
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

function createStyles(theme: AppTheme) {
  return StyleSheet.create({
    button: {
      paddingHorizontal: theme.space.medium,
      paddingVertical: theme.space.small,
      backgroundColor: theme.color.accent,
      borderRadius: theme.radius.button,
    },
    buttonDisabled: { opacity: 0.5 },
    buttonSecondary: { backgroundColor: theme.color.surface },
    buttonText: { color: theme.color.onAccent, fontWeight: '700' },
    buttonTextSecondary: { color: theme.color.text },
  });
}
