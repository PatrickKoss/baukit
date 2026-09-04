import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { buildWorker } from "./build-sw.mjs";

async function fixture(t, enabled) {
  const root = await mkdtemp(join(tmpdir(), "baukit-worker-template-"));
  t.after(() => rm(root, { recursive: true }));
  await mkdir(join(root, "web", "scripts"), { recursive: true });
  await writeFile(
    join(root, "baukit.toml"),
    `[capabilities]\nweb = true\npwa = ${enabled}\n\n[dependencies.baukit]\nsource = "registry"\n`,
  );
  return { root, webDirectory: join(root, "web") };
}

test("the generated check is a no-op while PWA support is not selected", async (t) => {
  const { webDirectory } = await fixture(t, false);

  assert.equal(
    await buildWorker({ webDirectory, check: true }),
    "PWA capability is disabled; no worker artifact is expected.",
  );
});

test("the build copies the published worker and the check detects drift", async (t) => {
  const { root, webDirectory } = await fixture(t, true);
  const source = join(root, "worker.js");
  const output = join(webDirectory, "public", "baukit-pwa-worker.js");
  await writeFile(source, "globalThis.BaukitPwa = {};\n");
  const options = { webDirectory, check: false, resolveWorker: () => source };

  await buildWorker(options);
  assert.equal(await readFile(output, "utf8"), "globalThis.BaukitPwa = {};\n");
  await assert.doesNotReject(buildWorker({ ...options, check: true }));

  await writeFile(output, "stale\n");
  await assert.rejects(
    buildWorker({ ...options, check: true }),
    /Worker artifact is stale/,
  );
});
