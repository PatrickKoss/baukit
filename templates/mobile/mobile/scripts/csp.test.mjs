import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findUnhashedInlineScripts, POLICY_FILE, sha256Base64, writePolicy } from "./csp.mjs";

const inline = "<script>window.boot = true;</script>";

async function withExport(html, check) {
  const directory = await mkdtemp(join(tmpdir(), "csp-export-"));
  try {
    await writeFile(join(directory, "index.html"), html);
    await check(directory);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

function directive(policy, name) {
  return policy
    .split(";")
    .map((part) => part.trim().split(" "))
    .find(([directiveName]) => directiveName === name)
    ?.slice(1);
}

test("the export policy allows runtime styles but never inline scripts", async () => {
  await withExport('<script src="/_expo/entry.js" defer></script>', async (directory) => {
    const policy = await writePolicy(directory, { EXPO_PUBLIC_API_URL: "https://api.example.test/v1" });

    assert.deepEqual(directive(policy, "style-src"), ["'self'", "'unsafe-inline'"]);
    assert.deepEqual(directive(policy, "script-src"), ["'self'"]);
    assert.deepEqual(directive(policy, "connect-src").slice(0, 2), ["'self'", "https://api.example.test"]);
    assert.equal(await readFile(join(directory, POLICY_FILE), "utf8"), `${policy}\n`);
  });
});

test("an inline script in the export gets a hash source", async () => {
  await withExport(inline, async (directory) => {
    const policy = await writePolicy(directory, {});

    assert.deepEqual(directive(policy, "script-src"), ["'self'", `'sha256-${sha256Base64("window.boot = true;")}'`]);
    assert.deepEqual(await findUnhashedInlineScripts(directory), []);
  });
});

test("the check fails on an inline script without a matching hash", async () => {
  await withExport('<script src="/_expo/entry.js" defer></script>', async (directory) => {
    await writePolicy(directory, {});
    await writeFile(join(directory, "index.html"), inline);

    assert.deepEqual(await findUnhashedInlineScripts(directory), [
      `index.html: 'sha256-${sha256Base64("window.boot = true;")}'`,
    ]);
  });
});
