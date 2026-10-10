{% raw %}
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import { Link } from "expo-router";
import { useTranslation } from "react-i18next";
import { useAuth } from "../../src/auth";
import { useTheme } from "../../src/theme";
import { ActionButton } from "../../src/action-button";
import { useAppPreferences } from "../../src/app-shell";
import { signOutWithPreferenceReset } from "../../src/preference-sign-out";

export default function ProfileScreen() {
  const [error, setError] = useState<string>();
  const auth = useAuth();
  const { t } = useTranslation("home");
  const { theme } = useTheme();
  const { resetPreferenceIdentity } = useAppPreferences();
  return (
    <View
      style={[
        styles.screen,
        {
          backgroundColor: theme.color.background,
          padding: theme.space.medium,
        },
      ]}
    >
      <Text accessibilityRole="header" style={{ color: theme.color.text }}>
        Profile
      </Text>
      <Text style={{ color: theme.color.text }}>
        Signed in as {auth.subject}.
      </Text>
      {auth.error === undefined ? null : (
        <Text accessibilityRole="alert" style={{ color: theme.color.error }}>
          {auth.error}
        </Text>
      )}
      {error === undefined ? null : (
        <Text accessibilityRole="alert" style={{ color: theme.color.error }}>
          {error}
        </Text>
      )}
      <ActionButton
        label="Sign out"
        onPress={() => {
          void signOutWithPreferenceReset({
            resetPreferenceIdentity,
            signOut: auth.signOut,
          }).catch((cause: unknown) => {
            setError(
              cause instanceof Error ? cause.message : "Could not sign out.",
            );
          });
        }}
      />
      <Link
        href="/delete-profile"
        accessibilityRole="link"
        style={{ color: theme.color.text }}
      >
        {t("erasure.title")}
      </Link>
    </View>
  );
}
const styles = StyleSheet.create({ screen: { flex: 1, gap: 16 } });
{% endraw -%}
