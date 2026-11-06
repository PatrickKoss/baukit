import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

const mobileRoot = join(import.meta.dirname, "..");
const localAdapter = join(mobileRoot, "src", "sign-in", "local.ts");
const productionAdapter = join(mobileRoot, "src", "sign-in", "production.ts");
const localIssuer = "http://localhost:{{ context.keycloak_host_port }}/realms/{{ context.app_name }}";
const variables = ["NODE_ENV", "BAUKIT_QA_BUILD", "EXPO_PUBLIC_LOCAL_OIDC_ISSUER", "EXPO_PUBLIC_API_URL"];
let loads = 0;

async function withEnvironment(environment, load) {
  const saved = Object.fromEntries(variables.map((name) => [name, process.env[name]]));
  for (const name of variables) delete process.env[name];
  Object.assign(process.env, environment);
  try {
    loads += 1;
    return await load(loads);
  } finally {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
}

const cases = [
  ["development", { NODE_ENV: "development", EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer }, true],
  ["production", { NODE_ENV: "production", EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer }, false],
  ["production QA", { NODE_ENV: "production", BAUKIT_QA_BUILD: "1", EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer }, true],
  ["production with a QA flag other than 1", { NODE_ENV: "production", BAUKIT_QA_BUILD: "true", EXPO_PUBLIC_LOCAL_OIDC_ISSUER: localIssuer }, false],
];

for (const [name, environment, local] of cases) {
  test(`a ${name} build ${local ? "resolves" : "never resolves"} the local sign-in adapter`, async () => {
    const config = await withEnvironment(environment, async (key) => (await import(`../metro.config.js?case=${key}`)).default);
    const delegate = (_context, moduleName) => ({
      type: "sourceFile",
      filePath: moduleName === "./sign-in/local" ? localAdapter : moduleName,
    });
    const resolveRequest = (moduleName) => config.resolver.resolveRequest({ resolveRequest: delegate }, moduleName, "ios");
    assert.deepEqual(resolveRequest("./sign-in/local"), {
      type: "sourceFile",
      filePath: local ? localAdapter : productionAdapter,
    });
    assert.deepEqual(resolveRequest("expo-router"), { type: "sourceFile", filePath: "expo-router" });
  });

  test(`a ${name} app config ${local ? "carries" : "drops"} the local realm and no default API URL`, async () => {
    const { extra } = await withEnvironment(environment, async (key) =>
      (await import(`../app.config.ts?case=${key}`)).default({ config: {} }),
    );
    assert.equal(extra.localOidcIssuer, local ? localIssuer : undefined);
    assert.equal(extra.apiBaseUrl, undefined);
  });
}

test("a configured API URL reaches the app config", async () => {
  const { extra } = await withEnvironment(
    { NODE_ENV: "production", EXPO_PUBLIC_API_URL: "https://api.example.test" },
    async (key) => (await import(`../app.config.ts?case=${key}`)).default({ config: {} }),
  );
  assert.equal(extra.apiBaseUrl, "https://api.example.test");
});
