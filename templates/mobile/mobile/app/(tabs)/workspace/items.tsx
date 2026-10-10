{% raw %}
import { StyleSheet, View } from "react-native";
import TodayScreen from "../index";
import { WorkspacePicker } from "../../../src/navigation-shell";
export default function ItemsScreen() {
  return (
    <View style={styles.screen}>
      <WorkspacePicker />
      <TodayScreen />
    </View>
  );
}
const styles = StyleSheet.create({ screen: { flex: 1 } });
{% endraw -%}
