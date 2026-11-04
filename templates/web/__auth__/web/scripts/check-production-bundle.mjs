import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const LOCAL_SIGN_IN_MARKERS = [
  { name: "Keycloak adapter", pattern: /keycloak|local-oidc/iu },
  { name: "local realm URL", pattern: /\/realms\//u },
  { name: "loopback URL", pattern: /https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])/iu },
];

export function findLocalSignInMarkers(text) {
  return LOCAL_SIGN_IN_MARKERS.filter(({ pattern }) => pattern.test(text)).map(({ name }) => name);
}

async function bundleFiles(directory) {
  const entries = await readdir(directory, { recursive: true, withFileTypes: true });
  return entries
    .filter((entry) => entry.isFile() && /\.(?:js|mjs|html|css|map|json)$/u.test(entry.name))
    .map((entry) => join(entry.parentPath, entry.name));
}

export async function scanBundle(directory) {
  const findings = [];
  for (const file of await bundleFiles(directory)) {
    for (const marker of findLocalSignInMarkers(await readFile(file, "utf8"))) {
      findings.push(`${relative(directory, file)}: ${marker}`);
    }
  }
  return findings;
}

async function main() {
  const webDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");
  const outDir = await mkdtemp(join(tmpdir(), "production-bundle-"));
  try {
    const build = spawnSync(
      join(webDirectory, "node_modules", ".bin", "vite"),
      ["build", "--mode", "production", "--outDir", outDir, "--emptyOutDir", "--logLevel", "warn"],
      {
        cwd: webDirectory,
        stdio: "inherit",
        env: { ...process.env, VITE_API_URL: process.env.VITE_API_URL ?? "https://api.example.invalid" },
      },
    );
    if (build.status !== 0) throw new Error("production build failed");
    const findings = await scanBundle(outDir);
    if (findings.length > 0) {
      console.error(`production bundle carries local sign-in code:\n${findings.join("\n")}`);
      process.exitCode = 1;
      return;
    }
    console.log("production bundle carries no Keycloak adapter, local realm URL or loopback URL");
  } finally {
    await rm(outDir, { recursive: true, force: true });
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
