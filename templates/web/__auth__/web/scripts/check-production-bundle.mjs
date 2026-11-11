import { randomUUID } from "node:crypto";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "vite";

const LOCAL_SIGN_IN_MARKERS = [
  {
    name: "Keycloak adapter",
    pattern: /keycloak|local-oidc/iu,
    anywhere: true,
  },
  { name: "local realm URL", pattern: /\/realms\//u, anywhere: true },
  {
    name: "loopback URL",
    pattern: /https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])/iu,
    anywhere: false,
  },
];

const ENV_CANARY_KEY = "VITE_BUNDLE_CHECK_CANARY";
const TEXT_ASSET = /\.(?:html|css|json|map)$/u;
const SOURCE_MAP = /\.map$/u;

export function isDependencyModule(id) {
  return id.replaceAll("\\", "/").includes("/node_modules/");
}

export function findLocalSignInMarkers(
  text,
  envCanary,
  { dependency = false } = {},
) {
  const markers = LOCAL_SIGN_IN_MARKERS.filter(
    ({ pattern, anywhere }) => (anywhere || !dependency) && pattern.test(text),
  ).map(({ name }) => name);
  if (envCanary !== undefined && text.includes(envCanary))
    markers.push("inlined Vite env object");
  return markers;
}

export function findBundleFindings(bundle, envCanary) {
  const findings = [];
  for (const [fileName, output] of Object.entries(bundle)) {
    if (output.type === "chunk") {
      for (const [id, module] of Object.entries(output.modules)) {
        const dependency = isDependencyModule(id);
        for (const marker of findLocalSignInMarkers(
          module.code ?? "",
          envCanary,
          { dependency },
        )) {
          findings.push(`${fileName}: ${id}: ${marker}`);
        }
      }
      continue;
    }
    if (!TEXT_ASSET.test(fileName)) continue;
    const text =
      typeof output.source === "string"
        ? output.source
        : Buffer.from(output.source).toString("utf8");
    for (const marker of findLocalSignInMarkers(text, envCanary, {
      dependency: SOURCE_MAP.test(fileName),
    })) {
      findings.push(`${fileName}: ${fileName}: ${marker}`);
    }
  }
  return findings;
}

function bundleScanPlugin(envCanary, findings) {
  return {
    name: "production-bundle-scan",
    generateBundle(_options, bundle) {
      findings.push(...findBundleFindings(bundle, envCanary));
    },
  };
}

async function main() {
  const webDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");
  const outDir = await mkdtemp(join(tmpdir(), "production-bundle-"));
  const envCanary = `canary-${randomUUID()}`;
  const findings = [];
  process.env.VITE_API_URL ??= "https://api.example.invalid";
  process.env[ENV_CANARY_KEY] = envCanary;
  try {
    await build({
      root: webDirectory,
      mode: "production",
      logLevel: "warn",
      build: { outDir, emptyOutDir: true },
      plugins: [bundleScanPlugin(envCanary, findings)],
    });
    if (findings.length > 0) {
      console.error(
        `production bundle carries local sign-in code:\n${findings.join("\n")}`,
      );
      process.exitCode = 1;
      return;
    }
    console.log(
      "production bundle carries no Keycloak adapter, local realm URL, first-party loopback URL or inlined Vite env object",
    );
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
