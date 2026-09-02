import { useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import type { ThemePreference } from './app-preferences';
import { useTheme, type AppTheme } from './theme';

const modes: readonly {
  readonly label: string;
  readonly value: ThemePreference;
}[] = [
  { label: 'System', value: 'system' },
  { label: 'Light', value: 'light' },
  { label: 'Dark', value: 'dark' },
];

export function ThemeModeControl() {
  const { mode, setMode, theme } = useTheme();
  const styles = createStyles(theme);
  const [error, setError] = useState<string>();

  async function chooseMode(nextMode: ThemePreference): Promise<void> {
    setError(undefined);
    try {
      await setMode(nextMode);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : 'Could not save the color scheme.',
      );
    }
  }

  return (
    <View style={styles.field}>
      <Text style={styles.label}>Color scheme</Text>
      <View
        accessibilityHint="Choose how the app selects light or dark colors"
        accessibilityLabel="Color scheme"
        accessibilityRole="radiogroup"
        style={styles.options}
      >
        {modes.map((option) => {
          const selected = option.value === mode;
          const accessibilityState = { checked: selected };
          return (
            <Pressable
              accessibilityHint={`Use the ${option.label.toLowerCase()} color scheme`}
              accessibilityLabel={option.label}
              accessibilityRole="radio"
              accessibilityState={accessibilityState}
              key={option.value}
              onPress={() => {
                void chooseMode(option.value);
              }}
              style={[
                styles.option,
                selected ? styles.optionSelected : undefined,
              ]}
            >
              <Text
                style={[
                  styles.optionText,
                  selected ? styles.optionTextSelected : undefined,
                ]}
              >
                {option.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      {error === undefined ? null : (
        <Text accessibilityLiveRegion="assertive" style={styles.error}>
          {error}
        </Text>
      )}
    </View>
  );
}

function createStyles(theme: AppTheme) {
  return StyleSheet.create({
    field: { gap: theme.space.small },
    label: { color: theme.color.text, fontSize: 16, fontWeight: '600' },
    options: { flexDirection: 'row', gap: theme.space.small },
    option: {
      borderColor: theme.color.border,
      borderRadius: theme.radius.button,
      borderWidth: 1,
      paddingHorizontal: theme.space.medium,
      paddingVertical: theme.space.small,
    },
    optionSelected: {
      backgroundColor: theme.color.accent,
      borderColor: theme.color.accent,
    },
    optionText: { color: theme.color.text },
    optionTextSelected: { color: theme.color.onAccent, fontWeight: '700' },
    error: { color: theme.color.error },
  });
}
