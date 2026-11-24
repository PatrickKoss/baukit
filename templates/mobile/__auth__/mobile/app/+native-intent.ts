import { appScheme } from "../src/app-scheme";

export function redirectSystemPath({
  path,
}: {
  readonly path: string;
  readonly initial: boolean;
}): string {
  const route = path.split(/[?#]/, 1)[0];
  return route === `${appScheme()}://oauth` || route === "/oauth" ? "/" : path;
}
