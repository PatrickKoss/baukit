import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const BUNDLE_MARKERS = [
  { name: "Keycloak adapter", pattern: /keycloak|local-oidc/giu },
  { name: "local realm URL", pattern: /\/realms\//gu },
  { name: "secret API key (sk_)", pattern: /\bsk_(?:test|live)_[A-Za-z0-9]{16,}/gu },
];
const PRODUCT_MARKERS = [{ name: "loopback URL", pattern: /https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])/giu }];
const LOCAL_ADAPTER_SOURCE = /(?:^|\/)src\/sign-in\/local\.[cm]?[jt]sx?$/u;
const PLATFORMS = ["android", "ios"];
const SOURCE_MAP_COMMENT = /\n\/\/# sourceMappingURL=[^\n]*/u;
const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const VLQ_CONTINUATION = 32;
const VLQ_VALUE = 31;

function decodeVlq(segment) {
  const values = [];
  let value = 0;
  let shift = 0;
  for (const character of segment) {
    const digit = BASE64.indexOf(character);
    value += (digit & VLQ_VALUE) << shift;
    if (digit & VLQ_CONTINUATION) {
      shift += 5;
      continue;
    }
    values.push(value & 1 ? -(value >>> 1) : value >>> 1);
    value = 0;
    shift = 0;
  }
  return values;
}

export function decodeMappings(mappings) {
  let source = 0;
  return mappings.split(";").map((line) => {
    let column = 0;
    return line
      .split(",")
      .filter((segment) => segment !== "")
      .map((segment) => {
        const [columnDelta, sourceDelta] = decodeVlq(segment);
        column += columnDelta;
        if (sourceDelta === undefined) return { column };
        source += sourceDelta;
        return { column, source };
      });
  });
}

export function isProductSource(source) {
  return !source.includes("/node_modules/") || source.includes("/node_modules/@baukit/");
}

function sourceAt(map, lines, line, column) {
  const segment = lines[line]?.findLast((candidate) => candidate.column <= column);
  return segment?.source === undefined ? undefined : map.sources[segment.source];
}

function matches(text, markers) {
  return markers.flatMap(({ name, pattern }) => [...text.matchAll(pattern)].map((match) => ({ name, index: match.index })));
}

function position(text, index) {
  const before = text.slice(0, index);
  const line = before.split("\n").length - 1;
  return { line, column: index - before.lastIndexOf("\n") - 1 };
}

export function scanBundleText(text, map) {
  const findings = [...new Set(matches(text, BUNDLE_MARKERS).map(({ name }) => name))];
  if (map.sources.some((source) => LOCAL_ADAPTER_SOURCE.test(source))) findings.push("Keycloak adapter module");
  const lines = decodeMappings(map.mappings);
  for (const { name, index } of matches(text, PRODUCT_MARKERS)) {
    const { line, column } = position(text, index);
    const source = sourceAt(map, lines, line, column);
    if (source === undefined || isProductSource(source)) findings.push(`${name} from ${source ?? "unmapped code"}`);
  }
  return [...new Set(findings)];
}

export async function scanExport(directory) {
  const entries = await readdir(directory, { recursive: true, withFileTypes: true });
  const bundles = entries.filter((entry) => entry.isFile() && entry.name.endsWith(".js"));
  if (bundles.length === 0) throw new Error(`no JavaScript bundle in ${directory}`);
  const findings = [];
  for (const entry of bundles) {
    const file = join(entry.parentPath, entry.name);
    const map = JSON.parse(await readFile(`${file}.map`, "utf8").catch(() => {
      throw new Error(`missing source map for ${relative(directory, file)}`);
    }));
    const text = (await readFile(file, "utf8")).replace(SOURCE_MAP_COMMENT, "");
    for (const finding of scanBundleText(text, map)) {
      findings.push(`${relative(directory, file)}: ${finding}`);
    }
  }
  return findings;
}

function productionEnvironment() {
  const environment = {
    ...process.env,
    NODE_ENV: "production",
    EXPO_PUBLIC_API_URL: process.env.EXPO_PUBLIC_API_URL ?? "https://api.example.invalid",
  };
  delete environment.BAUKIT_QA_BUILD;
  delete environment.EXPO_PUBLIC_LOCAL_OIDC_ISSUER;
  return environment;
}

async function main() {
  const mobileDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");
  const outDir = await mkdtemp(join(tmpdir(), "production-bundle-"));
  try {
    for (const platform of PLATFORMS) {
      const build = spawnSync(
        join(mobileDirectory, "node_modules", ".bin", "expo"),
        ["export", "--platform", platform, "--no-bytecode", "--source-maps", "--output-dir", join(outDir, platform)],
        { cwd: mobileDirectory, stdio: ["ignore", "ignore", "inherit"], env: productionEnvironment() },
      );
      if (build.status !== 0) throw new Error(`production ${platform} export failed`);
    }
    const findings = await scanExport(outDir);
    if (findings.length > 0) {
      console.error(`production bundle carries local sign-in code or a secret key:\n${findings.join("\n")}`);
      process.exitCode = 1;
      return;
    }
    console.log(
      `production ${PLATFORMS.join(" and ")} bundles carry no Keycloak adapter, local realm URL, product loopback URL or secret API key`,
    );
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
