import { redirectSystemPath } from "../app/+native-intent";
import { PRODUCT_NAME } from "./product";

describe("OIDC native callback routing", () => {
  it.each([true, false])(
    "routes callbacks to the root, initial=%s",
    (initial) => {
      for (const path of [
        `${PRODUCT_NAME}://oauth?state=known&code=authorization-code`,
        `${PRODUCT_NAME}://oauth?error=access_denied`,
        `${PRODUCT_NAME}://oauth#response`,
        "/oauth?state=known&code=authorization-code",
      ]) {
        expect(redirectSystemPath({ path, initial })).toBe("/");
      }
    },
  );

  it.each([
    `${PRODUCT_NAME}://settings/profile?tab=data`,
    `${PRODUCT_NAME}://workspace/items`,
    `${PRODUCT_NAME}://oauth-settings`,
    `/settings?next=${PRODUCT_NAME}://oauth`,
    "another-product://oauth?code=authorization-code",
    "/oauth/another-route",
    "not a URL",
    "",
  ])("preserves other links: %s", (path) => {
    expect(redirectSystemPath({ path, initial: false })).toBe(path);
  });
});
