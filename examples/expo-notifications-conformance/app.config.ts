import type { ExpoConfig } from "expo/config";

const config: ExpoConfig = {
  name: "Baukit Notifications Conformance",
  slug: "baukit-notifications-conformance",
  version: "0.0.0",
  orientation: "portrait",
  userInterfaceStyle: "automatic",
  android: {
    package: "dev.baukit.notificationsconformance",
  },
  ios: {
    bundleIdentifier: "dev.baukit.notificationsconformance",
    supportsTablet: true,
  },
};

export default config;
