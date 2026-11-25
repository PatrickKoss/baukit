import { appScheme } from "../src/app-scheme";
import { OIDC_REDIRECT_PATH } from "../src/product";

export function redirectSystemPath({
  path,
}: {
  readonly path: string;
  readonly initial: boolean;
}): string {
  const route = path.split(/[?#]/, 1)[0];
  return route === `${appScheme()}://${OIDC_REDIRECT_PATH}` || route === `/${OIDC_REDIRECT_PATH}` ? "/" : path;
}
