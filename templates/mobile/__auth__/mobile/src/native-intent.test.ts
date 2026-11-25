import { redirectSystemPath } from "../app/+native-intent";
import { OIDC_REDIRECT_PATH, PRODUCT_NAME } from "./product";

describe("OIDC native callback routing", () => {
  it.each([true, false])(
    "routes callbacks to the root, initial=%s",
    (initial) => {
      for (const path of [
        `${PRODUCT_NAME}://${OIDC_REDIRECT_PATH}?state=known&code=authorization-code`,
        `${PRODUCT_NAME}://${OIDC_REDIRECT_PATH}?error=access_denied`,
        `${PRODUCT_NAME}://${OIDC_REDIRECT_PATH}#response`,
        `/${OIDC_REDIRECT_PATH}?state=known&code=authorization-code`,
      ]) {
        expect(redirectSystemPath({ path, initial })).toBe("/");
      }
    },
  );

  it.each([
    `${PRODUCT_NAME}://settings/profile?tab=data`,
    `${PRODUCT_NAME}://workspace/items`,
    `${PRODUCT_NAME}://${OIDC_REDIRECT_PATH}-settings`,
    `/settings?next=${PRODUCT_NAME}://oauth`,
    "another-product://oauth?code=authorization-code",
    `/${OIDC_REDIRECT_PATH}/another-route`,
    "not a URL",
    "",
  ])("preserves other links: %s", (path) => {
    expect(redirectSystemPath({ path, initial: false })).toBe(path);
  });
});
