#!/usr/bin/env node
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  checkContrast,
  checkSemanticContrastMatrix,
  DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS,
  exampleTokens,
  parseTokens,
  toCssVariables,
} from "@baukit/ui-tokens";

async function readIfPresent(path) {
  try {
    return await readFile(path, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

function contrastFailures(tokens) {
  return [
    ...checkContrast(tokens).map(
      ({ foreground, background, theme, ratio, requiredRatio }) =>
        `${theme}: ${foreground} on ${background} is ${ratio.toFixed(2)}:1, needs ${requiredRatio}:1`,
    ),
    ...checkSemanticContrastMatrix(tokens, DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS).map(
      ({ foregroundRole, backgroundRole, theme, achievedRatio, minimumRatio }) =>
        `${theme}: ${foregroundRole} on ${backgroundRole} is ${achievedRatio.toFixed(2)}:1, needs ${minimumRatio}:1`,
    ),
  ];
}

export async function generateTokens({ sourceDirectory }) {
  const productTokens = await readIfPresent(join(sourceDirectory, "design-tokens.json"));
  let tokens = exampleTokens;
  if (productTokens !== null) {
    tokens = parseTokens(JSON.parse(productTokens));
    const failures = contrastFailures(tokens);
    if (failures.length > 0) {
      throw new Error(`Design tokens fail WCAG contrast:\n${failures.join("\n")}`);
    }
  }
  const extraCss = await readIfPresent(join(sourceDirectory, "tokens.extra.css"));
  const css = toCssVariables(tokens);
  const output = extraCss === null ? css : `${css.trimEnd()}\n\n${extraCss.trimEnd()}\n`;
  await writeFile(join(sourceDirectory, "tokens.css"), output, "utf8");
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    await generateTokens({ sourceDirectory: fileURLToPath(new URL("../src", import.meta.url)) });
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
