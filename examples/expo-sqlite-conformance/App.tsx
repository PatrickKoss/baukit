import { useEffect, useState } from "react";
import { SafeAreaView, StyleSheet, Text } from "react-native";

import { runConformance } from "./src/conformance";

export const PASS_MARKER = "BAUKIT_SQLITE_CONFORMANCE_PASS";
export const FAIL_MARKER = "BAUKIT_SQLITE_CONFORMANCE_FAIL";

export default function App() {
  const [status, setStatus] = useState("Running real Expo SQLite conformance…");

  useEffect(() => {
    let mounted = true;
    void runConformance()
      .then((result) => {
        const message = `${PASS_MARKER} ${JSON.stringify(result)}`;
        console.log(message);
        if (mounted) setStatus(message);
      })
      .catch((cause: unknown) => {
        const detail = cause instanceof Error ? cause.message : String(cause);
        const message = `${FAIL_MARKER} ${detail}`;
        console.error(message);
        if (mounted) setStatus(message);
      });
    return () => {
      mounted = false;
    };
  }, []);

  return (
    <SafeAreaView style={styles.container}>
      <Text selectable>{status}</Text>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    justifyContent: "center",
    padding: 24,
  },
});
