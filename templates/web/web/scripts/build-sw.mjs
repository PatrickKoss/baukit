#!/usr/bin/env node
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const WORKER_FILE = "baukit-pwa-worker.js";

function pwaEnabled(manifest) {
  let inCapabilities = false;
  for (const line of manifest.split("\n")) {
    const value = line.trim();
    if (value.startsWith("[") && value.endsWith("]")) {
      inCapabilities = value === "[capabilities]";
      continue;
    }
    if (inCapabilities && /^pwa\s*=\s*true(?:\s+#.*)?$/.test(value)) {
      return true;
    }
  }
  return false;
}

async function readIfPresent(path) {
  try {
    return await readFile(path);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

export async function buildWorker({
  webDirectory,
  outputDirectory = "public",
  check,
  resolveWorker = () => import.meta.resolve("@baukit/pwa-web/worker"),
}) {
  const manifestPath = resolve(webDirectory, "..", "baukit.toml");
  const output = join(outputDirectory, WORKER_FILE);
  const outputPath = join(webDirectory, output);
  const manifest = await readFile(manifestPath, "utf8");

  if (!pwaEnabled(manifest)) {
    const existing = await readIfPresent(outputPath);
    if (existing !== null) {
      throw new Error(`${output} exists while capabilities.pwa is false`);
    }
    return "PWA capability is disabled; no worker artifact is expected.";
  }

  if (
    outputDirectory !== "public" &&
    (await readIfPresent(join(webDirectory, outputDirectory, "index.html"))) === null
  ) {
    throw new Error(
      "Web export is missing. Run: corepack pnpm exec expo export --platform web",
    );
  }

  const resolvedWorker = resolveWorker();
  const workerPath = resolvedWorker.startsWith("file:")
    ? fileURLToPath(resolvedWorker)
    : resolvedWorker;
  const expected = await readFile(workerPath);

  if (check) {
    const actual = await readIfPresent(outputPath);
    if (actual === null || !actual.equals(expected)) {
      throw new Error(
        `Worker artifact is stale. Run: corepack pnpm run build:sw`,
      );
    }
    return `Worker artifact is current at ${output}.`;
  }

  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, expected);
  return `Copied @baukit/pwa-web/worker to ${output}.`;
}

const invokedPath =
  process.argv[1] === undefined
    ? null
    : pathToFileURL(resolve(process.argv[1])).href;
if (invokedPath === import.meta.url) {
  const webDirectory = dirname(dirname(fileURLToPath(import.meta.url)));
  try {
    const { values } = parseArgs({
      options: {
        check: { type: "boolean", default: false },
        "output-dir": { type: "string", default: "public" },
      },
    });
    console.log(
      await buildWorker({
        webDirectory,
        outputDirectory: values["output-dir"],
        check: values.check,
      }),
    );
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
