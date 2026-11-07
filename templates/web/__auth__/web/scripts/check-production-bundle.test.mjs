import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findLocalSignInMarkers, scanBundle } from "./check-production-bundle.mjs";

test("the scanner names each kind of local sign-in leftover", () => {
  assert.deepEqual(findLocalSignInMarkers("Sign in with local Keycloak"), ["Keycloak adapter"]);
  assert.deepEqual(findLocalSignInMarkers("product:local-oidc:tokens"), ["Keycloak adapter"]);
  assert.deepEqual(findLocalSignInMarkers("https://login.example/realms/product"), ["local realm URL"]);
  assert.deepEqual(findLocalSignInMarkers("fetch('http://127.0.0.1:8080/me')"), ["loopback URL"]);
  assert.deepEqual(findLocalSignInMarkers("['localhost','127.0.0.1'].includes(host)"), []);
});

test("the scanner reports an inlined Vite env object by its canary value", () => {
  assert.deepEqual(findLocalSignInMarkers('const env = {"VITE_X":"canary-1"}', "canary-1"), ["inlined Vite env object"]);
  assert.deepEqual(findLocalSignInMarkers("const key = 'pk_test'", "canary-1"), []);
});

test("the bundle scan reports the file and marker", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "bundle-scan-"));
  t.after(() => rm(directory, { recursive: true }));
  await writeFile(join(directory, "clean.js"), "export const api = 'https://api.example.test';");
  assert.deepEqual(await scanBundle(directory), []);
  await writeFile(join(directory, "leak.js"), "const issuer = 'http://localhost:8180/realms/app';");
  assert.deepEqual(await scanBundle(directory), ["leak.js: local realm URL", "leak.js: loopback URL"]);
});
