#!/usr/bin/env node
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";

import { policyOrigins } from "../src/csp/origins.ts";
import {
  contentSecurityPolicy,
  hashSource,
  inlineScripts,
  unhashedInlineScripts,
} from "../src/csp/policy.ts";

export const POLICY_FILE = "csp.txt";

export function sha256Base64(text) {
  return createHash("sha256").update(text, "utf8").digest("base64");
}

async function htmlFiles(directory) {
  const entries = await readdir(directory, { recursive: true, withFileTypes: true });
  return entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".html"))
    .map((entry) => join(entry.parentPath, entry.name));
}

/** Writes `<outDir>/csp.txt` for the Expo web export, hashing every inline script. */
export async function writePolicy(outDir, env) {
  const pages = await Promise.all((await htmlFiles(outDir)).map((file) => readFile(file, "utf8")));
  const scriptHashes = [
    ...new Set(pages.flatMap((html) => inlineScripts(html).map((script) => hashSource(sha256Base64(script))))),
  ];
  // react-native-web injects <style> elements at runtime (ADR 0006 section 7).
  const policy = contentSecurityPolicy({ styles: "runtime-injected", origins: policyOrigins(env), scriptHashes });
  await writeFile(join(outDir, POLICY_FILE), `${policy}\n`);
  return policy;
}

/** Lists every exported HTML file with an inline script that `csp.txt` does not hash. */
export async function findUnhashedInlineScripts(outDir) {
  const policy = await readFile(join(outDir, POLICY_FILE), "utf8");
  const findings = [];
  for (const file of await htmlFiles(outDir)) {
    const missing = unhashedInlineScripts(await readFile(file, "utf8"), policy, sha256Base64);
    findings.push(...missing.map((source) => `${relative(outDir, file)}: ${source}`));
  }
  return findings;
}

if (import.meta.main) {
  const outDir = process.argv[2] ?? "dist";
  if (existsSync(".env")) {
    process.loadEnvFile(".env");
  }
  await writePolicy(outDir, process.env);
  const findings = await findUnhashedInlineScripts(outDir);
  if (findings.length > 0) {
    console.error(`Inline scripts without a matching ${POLICY_FILE} hash:\n${findings.join("\n")}`);
    process.exitCode = 1;
  } else {
    console.log(`Wrote ${join(outDir, POLICY_FILE)}; every inline script has a hash.`);
  }
}
