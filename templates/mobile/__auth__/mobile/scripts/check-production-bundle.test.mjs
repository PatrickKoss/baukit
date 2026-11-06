import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  decodeMappings,
  developerEnvironment,
  isProductSource,
  publicFiles,
  scanBundleText,
  scanExport,
} from "./check-production-bundle.mjs";

const appSource = "/src/api.ts";
const librarySource = "/node_modules/react-native/Libraries/Core/Devtools/getDevServer.js";

function mapFor(sources, mappings) {
  return { version: 3, sources, mappings };
}

test("the mapping decoder tracks columns and sources across segments", () => {
  assert.deepEqual(decodeMappings("AAAA,UCAA;AAAA"), [
    [
      { column: 0, source: 0 },
      { column: 10, source: 1 },
    ],
    [{ column: 0, source: 1 }],
  ]);
});

test("only Baukit and product code count as product sources", () => {
  assert.equal(isProductSource(appSource), true);
  assert.equal(isProductSource("/node_modules/.pnpm/x/node_modules/@baukit/api-runtime/dist/index.js"), true);
  assert.equal(isProductSource(librarySource), false);
});

test("a loopback URL fails only when product code emits it", () => {
  const text = "a='http://localhost:8081/';b='http://localhost:8080'";
  const libraryThenApp = mapFor([librarySource, appSource], "AAAA,0BCAA");
  assert.deepEqual(scanBundleText(text, libraryThenApp), [`loopback URL from ${appSource}`]);
  assert.deepEqual(scanBundleText(text, mapFor([librarySource], "AAAA")), []);
  assert.deepEqual(scanBundleText(text, mapFor([librarySource], "")), ["loopback URL from unmapped code"]);
});

test("Keycloak markers, realm URLs and the local adapter module fail anywhere", () => {
  const library = mapFor([librarySource], "AAAA");
  assert.deepEqual(scanBundleText("label='Sign in with local Keycloak'", library), ["Keycloak adapter"]);
  assert.deepEqual(scanBundleText("i='https://login.example/realms/app'", library), ["local realm URL"]);
  assert.deepEqual(scanBundleText("x=1", mapFor(["/src/sign-in/local.ts"], "AAAA")), ["Keycloak adapter module"]);
});

test("a WorkOS or Clerk secret API key fails anywhere", () => {
  const library = mapFor([librarySource], "AAAA");
  const key = ["sk", "test", "placeholderplaceholder0000"].join("_");
  assert.deepEqual(scanBundleText(`k='${key}'`, library), ["secret API key (sk_)"]);
  assert.deepEqual(scanBundleText(`k='${key.replace("test", "live")}'`, mapFor([appSource], "AAAA")), [
    "secret API key (sk_)",
  ]);
  assert.deepEqual(scanBundleText("disk_test_placeholderplaceholder0000;sk_test_short", library), []);
});

test("the export scan reads each bundle with its source map", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "bundle-scan-"));
  t.after(() => rm(directory, { recursive: true }));
  await writeFile(join(directory, "entry.js"), "const api = 'https://api.example.test';");
  await writeFile(join(directory, "entry.js.map"), JSON.stringify(mapFor([appSource], "AAAA")));
  assert.deepEqual(await scanExport(directory), []);
  await writeFile(
    join(directory, "entry.js"),
    "const api = 'https://api.example.test';\n//# sourceMappingURL=http://localhost:8081/entry.js.map",
  );
  assert.deepEqual(await scanExport(directory), []);
  await writeFile(join(directory, "entry.js"), "const issuer = 'http://localhost:8180/realms/app';");
  assert.deepEqual(await scanExport(directory), [
    "entry.js: local realm URL",
    `entry.js: loopback URL from ${appSource}`,
  ]);
});

test("a copied public file without a source map is scanned as unmapped code", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "bundle-public-"));
  t.after(() => rm(root, { recursive: true }));
  const exported = join(root, "export");
  await mkdir(join(root, "public", "vendor"), { recursive: true });
  await mkdir(join(exported, "vendor"), { recursive: true });
  await writeFile(join(root, "public", "vendor", "worker.js"), "");
  await writeFile(join(exported, "vendor", "worker.js"), "fetch('http://localhost:8080/api');");
  const copied = await publicFiles(join(root, "public"));
  assert.deepEqual([...copied], [join("vendor", "worker.js")]);
  assert.deepEqual(await scanExport(exported, copied), [
    `${join("vendor", "worker.js")}: loopback URL from unmapped code`,
  ]);
  await assert.rejects(scanExport(exported), /missing source map for vendor\/worker\.js/u);
  assert.deepEqual(await publicFiles(join(root, "missing")), new Set());
});

test("without a mobile/.env the check exports with the example values a developer copies", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "bundle-env-"));
  t.after(() => rm(directory, { recursive: true }));
  await writeFile(join(directory, ".env.example"), "EXPO_PUBLIC_LOCAL_OIDC_ISSUER=http://localhost:8180/realms/app\n");
  assert.deepEqual(await developerEnvironment(directory), {
    EXPO_PUBLIC_LOCAL_OIDC_ISSUER: "http://localhost:8180/realms/app",
  });
  await writeFile(join(directory, ".env"), "EXPO_PUBLIC_API_URL=http://localhost:8080\n");
  assert.deepEqual(await developerEnvironment(directory), {});
});
