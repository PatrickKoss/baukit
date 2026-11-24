import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

const mobileRoot = join(import.meta.dirname, "..");
const localAdapter = join(mobileRoot, "src", "sign-in", "local.ts");
const productionAdapter = join(mobileRoot, "src", "sign-in", "production.ts");
const labAdapter = join(mobileRoot, "src", "sign-in", "lab.ts");
const variables = ["NODE_ENV", "BAUKIT_QA_BUILD", "APP_VARIANT"];
let loads = 0;

async function withEnvironment(environment, load) {
  const saved = process.env;
  process.env = {
    ...Object.fromEntries(Object.entries(saved).filter(([name]) => !variables.includes(name))),
    ...environment,
  };
  try {
    loads += 1;
    return await load(loads);
  } finally {
    process.env = saved;
  }
}

const cases = [
  ["development", { NODE_ENV: "development" }, localAdapter],
  ["production", { NODE_ENV: "production" }, productionAdapter],
  ["production QA", { NODE_ENV: "production", BAUKIT_QA_BUILD: "1" }, localAdapter],
  ["production with a QA flag other than 1", { NODE_ENV: "production", BAUKIT_QA_BUILD: "true" }, productionAdapter],
  ["production with an empty APP_VARIANT", { NODE_ENV: "production", APP_VARIANT: "" }, productionAdapter],
  ["production dev variant", { NODE_ENV: "production", APP_VARIANT: "dev" }, labAdapter],
  ["production lab variant", { NODE_ENV: "production", APP_VARIANT: "lab" }, labAdapter],
  ["development dev variant", { NODE_ENV: "development", APP_VARIANT: "dev" }, labAdapter],
];

for (const [name, environment, adapter] of cases) {
  test(`a ${name} build resolves the ${adapter.split("/").at(-1)} sign-in adapter`, async () => {
    const config = await withEnvironment(environment, async (key) => (await import(`../metro.config.js?case=${key}`)).default);
    const delegate = (_context, moduleName) => ({
      type: "sourceFile",
      filePath: moduleName === "./sign-in/local" ? localAdapter : moduleName,
    });
    const resolveRequest = (moduleName) => config.resolver.resolveRequest({ resolveRequest: delegate }, moduleName, "ios");
    assert.deepEqual(resolveRequest("./sign-in/local"), {
      type: "sourceFile",
      filePath: adapter,
    });
    assert.deepEqual(resolveRequest("expo-router"), { type: "sourceFile", filePath: "expo-router" });
  });
}
