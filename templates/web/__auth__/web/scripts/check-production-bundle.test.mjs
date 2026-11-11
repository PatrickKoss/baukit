import assert from "node:assert/strict";
import { test } from "node:test";

import {
  findBundleFindings,
  findLocalSignInMarkers,
} from "./check-production-bundle.mjs";

const dependencyId =
  "/repo/web/node_modules/.pnpm/router-core/node_modules/@tanstack/router-core/dist/router.js";
const productId = "/repo/web/src/api-base-url.ts";

function chunk(modules) {
  return {
    type: "chunk",
    modules: Object.fromEntries(
      Object.entries(modules).map(([id, code]) => [id, { code }]),
    ),
  };
}

test("the scanner names each kind of local sign-in leftover", () => {
  assert.deepEqual(findLocalSignInMarkers("Sign in with local Keycloak"), [
    "Keycloak adapter",
  ]);
  assert.deepEqual(findLocalSignInMarkers("product:local-oidc:account"), [
    "Keycloak adapter",
  ]);
  assert.deepEqual(
    findLocalSignInMarkers("https://login.example/realms/product"),
    ["local realm URL"],
  );
  assert.deepEqual(
    findLocalSignInMarkers("fetch('http://127.0.0.1:8080/me')"),
    ["loopback URL"],
  );
  assert.deepEqual(
    findLocalSignInMarkers("['localhost','127.0.0.1'].includes(host)"),
    [],
  );
});

test("the scanner reports an inlined Vite env object by its canary value", () => {
  assert.deepEqual(
    findLocalSignInMarkers('const env = {"VITE_X":"canary-1"}', "canary-1"),
    ["inlined Vite env object"],
  );
  assert.deepEqual(
    findLocalSignInMarkers("const key = 'pk_test'", "canary-1"),
    [],
  );
});

test("a dependency module carrying a loopback URL passes", () => {
  const bundle = {
    "vendor.js": chunk({ [dependencyId]: 'this.origin = "http://localhost"' }),
  };
  assert.deepEqual(findBundleFindings(bundle, "canary-1"), []);
});

test("a first-party module carrying a loopback URL fails with chunk, module and marker", () => {
  const bundle = {
    "api.js": chunk({ [productId]: 'return "http://localhost:8080"' }),
  };
  assert.deepEqual(findBundleFindings(bundle, "canary-1"), [
    `api.js: ${productId}: loopback URL`,
  ]);
});

test("a dependency module carrying keycloak still fails", () => {
  const bundle = { "vendor.js": chunk({ [dependencyId]: "keycloak-js" }) };
  assert.deepEqual(findBundleFindings(bundle, "canary-1"), [
    `vendor.js: ${dependencyId}: Keycloak adapter`,
  ]);
});

test("the canary fails in a dependency module and a first-party module", () => {
  const bundle = {
    "a.js": chunk({ [dependencyId]: "x canary-1" }),
    "b.js": chunk({ [productId]: "y canary-1" }),
  };
  assert.deepEqual(findBundleFindings(bundle, "canary-1"), [
    `a.js: ${dependencyId}: inlined Vite env object`,
    `b.js: ${productId}: inlined Vite env object`,
  ]);
});

test("text assets are scanned and source maps ignore loopback URLs", () => {
  const bundle = {
    "index.html": { type: "asset", source: "<a href='http://localhost:1'>" },
    "a.js.map": { type: "asset", source: "http://localhost realms" },
    "logo.png": { type: "asset", source: new Uint8Array([1]) },
  };
  assert.deepEqual(findBundleFindings(bundle, "canary-1"), [
    "index.html: index.html: loopback URL",
  ]);
});
