{% raw %}
import type { ReactElement } from 'react';
import { Stack } from "expo-router";
export default function WorkspaceLayout(): ReactElement {
  return <Stack screenOptions={{ headerShown: false }} />;
}
{% endraw -%}
