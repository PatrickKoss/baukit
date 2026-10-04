import { PRODUCT_NAME } from "../src/product";

export function redirectSystemPath({
  path,
}: {
  readonly path: string;
  readonly initial: boolean;
}): string {
  const route = path.split(/[?#]/, 1)[0];
  return route === `${PRODUCT_NAME}://oauth` || route === "/oauth" ? "/" : path;
}
