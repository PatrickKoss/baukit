import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { assertQaBuildFlagFromShell, envFilesSettingQaBuildFlag } from "./qa-build-flag.mjs";

async function withEnvFiles(files, check) {
  const directory = await mkdtemp(join(tmpdir(), "qa-build-flag-"));
  try {
    for (const [name, contents] of Object.entries(files)) await writeFile(join(directory, name), contents);
    await check(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

test("env files without the QA flag pass", () =>
  withEnvFiles({ ".env": "EXPO_PUBLIC_API_URL=http://localhost:8080\nBAUKIT_QA_BUILD_NOTE=1\n" }, (directory) => {
    assert.deepEqual(envFilesSettingQaBuildFlag(directory), []);
    assert.doesNotThrow(() => assertQaBuildFlagFromShell(directory));
  }));

test("a QA flag in any env file Expo loads is refused", () =>
  withEnvFiles(
    { ".env": "BAUKIT_QA_BUILD=1\n", ".env.production.local": "  export BAUKIT_QA_BUILD=0\n" },
    (directory) => {
      assert.deepEqual(envFilesSettingQaBuildFlag(directory), [".env", ".env.production.local"]);
      assert.throws(() => assertQaBuildFlagFromShell(directory), /\.env, \.env\.production\.local sets BAUKIT_QA_BUILD/u);
    },
  ));
