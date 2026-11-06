import { readFileSync } from "node:fs";
import { join } from "node:path";

export const QA_BUILD_FLAG = "BAUKIT_QA_BUILD";
const EXPO_MODES = ["development", "production", "test"];
export const EXPO_ENV_FILES = [
  ".env",
  ".env.local",
  ...EXPO_MODES.flatMap((mode) => [`.env.${mode}`, `.env.${mode}.local`]),
];
const ASSIGNMENT = new RegExp(`^\\s*(?:export\\s+)?${QA_BUILD_FLAG}\\s*=`, "mu");

function readOptional(path) {
  try {
    return readFileSync(path, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return "";
    throw error;
  }
}

export function envFilesSettingQaBuildFlag(directory) {
  return EXPO_ENV_FILES.filter((name) => ASSIGNMENT.test(readOptional(join(directory, name))));
}

export function assertQaBuildFlagFromShell(directory) {
  const files = envFilesSettingQaBuildFlag(directory);
  if (files.length > 0) {
    throw new Error(
      `${files.join(", ")} sets ${QA_BUILD_FLAG}. Expo loads env files into every export, so a production bundle would sign in locally. Remove it; the QA scripts export the flag in their shell.`,
    );
  }
}
