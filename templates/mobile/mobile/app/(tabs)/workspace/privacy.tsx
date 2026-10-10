{% raw %}
import { StyleSheet, Text, View } from "react-native";
import { WorkspacePicker } from "../../../src/navigation-shell";
import { useTheme } from "../../../src/theme";
export default function PrivacyScreen() {
  const { theme } = useTheme();
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
      <WorkspacePicker />
      <Text accessibilityRole="header" style={{ color: theme.color.text }}>
        Analytics privacy
      </Text>
      <Text style={{ color: theme.color.text }}>
        Analytics starts with no consent. Use the privacy controls on Today to
        allow or deny analytics. The generated app sends no analytics events to
        a provider.
      </Text>
    </View>
  );
}
const styles = StyleSheet.create({ screen: { flex: 1, gap: 16 } });
{% endraw -%}
