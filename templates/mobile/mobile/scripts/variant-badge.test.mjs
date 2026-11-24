import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";

const { badgedIcon } = createRequire(import.meta.url)("../plugins/with-variant-badge.cjs");

test("the variant badge layers a dot over the product's own launcher icon", () => {
  const icon = badgedIcon("ic_launcher_round");
  assert.match(icon, /^<\?xml/u);
  assert.match(icon, /<item android:drawable="@mipmap\/ic_launcher_round" \/>/u);
  assert.match(icon, /<shape android:shape="oval">/u);
});
