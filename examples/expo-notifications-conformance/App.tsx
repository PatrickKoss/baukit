import { useEffect, useState } from "react";
import { SafeAreaView, StyleSheet, Text } from "react-native";

import {
  isPermissionGranted,
  runDenied,
  runGranted,
} from "./src/notifications";
import { runVectors, type SuiteResult } from "./src/vectors";

export const VECTORS_PASS_MARKER = "BAUKIT_HERMES_VECTORS_PASS";
export const GRANTED_PASS_MARKER = "BAUKIT_NOTIFICATIONS_GRANTED_PASS";
export const DENIED_PASS_MARKER = "BAUKIT_NOTIFICATIONS_DENIED_PASS";
export const MISMATCH_MARKER = "BAUKIT_HERMES_VECTOR_MISMATCH";
export const FAIL_MARKER = "BAUKIT_NOTIFICATIONS_CONFORMANCE_FAIL";

declare const HermesInternal: unknown;

function report(message: string, lines: string[]): void {
  console.log(message);
  lines.push(message);
}

function reportMismatches(
  suite: string,
  result: SuiteResult,
  lines: string[],
): void {
  for (const found of result.mismatches) {
    report(`${MISMATCH_MARKER} ${suite} ${JSON.stringify(found)}`, lines);
  }
}

async function runConformance(lines: string[]): Promise<void> {
  if (typeof HermesInternal === "undefined") {
    throw new Error("the JavaScript engine is not Hermes");
  }
  const vectors = runVectors();
  reportMismatches("zonedTime", vectors.zonedTime, lines);
  reportMismatches("notificationPlan", vectors.notificationPlan, lines);
  const mismatches =
    vectors.zonedTime.mismatches.length +
    vectors.notificationPlan.mismatches.length;
  if (mismatches > 0) {
    throw new Error(
      `${mismatches} vector(s) disagree with the fixtures inside Hermes`,
    );
  }
  const deviceZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  report(
    `${VECTORS_PASS_MARKER} ${JSON.stringify({
      zonedTime: vectors.zonedTime.passed,
      notificationPlan: vectors.notificationPlan.passed,
      deviceZone,
    })}`,
    lines,
  );

  if (await isPermissionGranted()) {
    report(
      `${GRANTED_PASS_MARKER} ${JSON.stringify(await runGranted())}`,
      lines,
    );
  } else {
    report(`${DENIED_PASS_MARKER} ${JSON.stringify(await runDenied())}`, lines);
  }
}

export default function App() {
  const [status, setStatus] = useState(
    "Running Hermes vectors and expo-notifications checks…",
  );

  useEffect(() => {
    let mounted = true;
    const lines: string[] = [];
    void runConformance(lines)
      .then(() => {
        if (mounted) setStatus(lines.join("\n"));
      })
      .catch((cause: unknown) => {
        const detail = cause instanceof Error ? cause.message : String(cause);
        const message = `${FAIL_MARKER} ${detail}`;
        console.error(message);
        if (mounted) setStatus([...lines, message].join("\n"));
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
