import type { ExpoConfig } from "expo/config";

const config: ExpoConfig = {
  name: "Baukit SQLite Conformance",
  slug: "baukit-sqlite-conformance",
  version: "0.0.0",
  orientation: "portrait",
  userInterfaceStyle: "automatic",
  android: {
    package: "dev.baukit.sqliteconformance",
  },
  ios: {
    bundleIdentifier: "dev.baukit.sqliteconformance",
    supportsTablet: true,
  },
};

export default config;
