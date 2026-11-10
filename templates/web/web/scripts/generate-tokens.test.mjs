import assert from "node:assert/strict";
import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { exampleTokens, toCssVariables } from "@baukit/ui-tokens";

import { generateTokens } from "./generate-tokens.mjs";

async function sourceDirectory(t) {
  const directory = await mkdtemp(join(tmpdir(), "baukit-tokens-template-"));
  t.after(() => rm(directory, { recursive: true }));
  return directory;
}

test("example tokens render when the product has no token source", async (t) => {
  const directory = await sourceDirectory(t);

  await generateTokens({ sourceDirectory: directory });

  assert.equal(await readFile(join(directory, "tokens.css"), "utf8"), toCssVariables(exampleTokens));
});

test("tokens.extra.css is appended after the generated variables", async (t) => {
  const directory = await sourceDirectory(t);
  await writeFile(join(directory, "tokens.extra.css"), ":root { --entry-sand: #c6a66f; }\n");

  await generateTokens({ sourceDirectory: directory });

  const css = await readFile(join(directory, "tokens.css"), "utf8");
  assert.ok(css.startsWith(toCssVariables(exampleTokens).trimEnd()));
  assert.ok(css.endsWith(":root { --entry-sand: #c6a66f; }\n"));
});

test("design-tokens.json replaces the example tokens", async (t) => {
  const directory = await sourceDirectory(t);
  const tokens = structuredClone(exampleTokens);
  tokens.color.action.primary.light = "#003a8c";
  await writeFile(join(directory, "design-tokens.json"), JSON.stringify(tokens));

  await generateTokens({ sourceDirectory: directory });

  assert.match(await readFile(join(directory, "tokens.css"), "utf8"), /#003a8c/);
});

test("design-tokens.json that misses WCAG contrast fails without writing output", async (t) => {
  const directory = await sourceDirectory(t);
  const tokens = structuredClone(exampleTokens);
  tokens.color.text.primary.light = "#fafafa";
  await writeFile(join(directory, "design-tokens.json"), JSON.stringify(tokens));

  await assert.rejects(generateTokens({ sourceDirectory: directory }), /fail WCAG contrast/);
  await assert.rejects(access(join(directory, "tokens.css")));
});
