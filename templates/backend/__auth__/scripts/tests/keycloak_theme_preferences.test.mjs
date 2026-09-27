import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const SCRIPT_URL = new URL(
  "../../keycloak/themes/baukit-accessible/login/resources/js/theme-preferences.js",
  import.meta.url,
);
const AUTH_URL = "https://keycloak.test/realms/app/protocol/openid-connect/auth";
const NONCE = "ab".repeat(32);
const DARK = "pf-v5-theme-dark";

class FakeClassList {
  constructor() {
    this.values = new Set();
    this.onChange = () => {};
  }

  contains(value) {
    return this.values.has(value);
  }

  toggle(value, force) {
    if (force) this.values.add(value);
    else this.values.delete(value);
    this.onChange();
  }
}

function fakePage(query) {
  const properties = new Map();
  const observers = [];
  const root = {
    classList: new FakeClassList(),
    dataset: {},
    style: {
      setProperty(name, value) {
        properties.set(name, value);
      },
    },
  };
  class MutationObserver {
    constructor(callback) {
      this.callback = callback;
    }

    observe(target, options) {
      observers.push({ target, options, callback: this.callback });
    }
  }
  root.classList.onChange = () => {
    for (const observer of observers) observer.callback([]);
  };
  const context = {
    document: { documentElement: root },
    location: { href: `${AUTH_URL}?${query}` },
    MutationObserver,
    URL,
    atob,
  };
  return { context, root, properties, observers };
}

async function runTheme(query) {
  const page = fakePage(query);
  const source = await readFile(SCRIPT_URL, "utf8");
  vm.runInNewContext(source, page.context);
  return page;
}

function clientData(state) {
  return Buffer.from(JSON.stringify({ ru: "app://oauth", st: state })).toString(
    "base64url",
  );
}

test("dark state pins the dark class against keycloak's media query", async () => {
  const page = await runTheme(`state=ap1.d.${NONCE}`);

  assert.equal(page.root.dataset.baukitTheme, "dark");
  assert.equal(page.root.classList.contains(DARK), true);
  assert.deepEqual([...page.observers[0].options.attributeFilter], ["class"]);

  page.root.classList.toggle(DARK, false);
  assert.equal(page.root.classList.contains(DARK), true);
});

test("light state removes the dark class", async () => {
  const page = fakePage(`state=ap1.l.${NONCE}`);
  page.root.classList.values.add(DARK);
  vm.runInNewContext(await readFile(SCRIPT_URL, "utf8"), page.context);

  assert.equal(page.root.dataset.baukitTheme, "light");
  assert.equal(page.root.classList.contains(DARK), false);
});

test("system state leaves keycloak's color scheme handling alone", async () => {
  const page = await runTheme(`state=ap1.s.${NONCE}`);

  assert.equal(page.root.dataset.baukitTheme, undefined);
  assert.equal(page.observers.length, 0);
});

test("colors become custom properties with readable text", async () => {
  const page = await runTheme(`state=ap1.l.0a7cff.FFE066.${NONCE}`);

  assert.equal(page.properties.get("--baukit-auth-primary"), "#0A7CFF");
  assert.equal(page.properties.get("--baukit-auth-secondary"), "#FFE066");
  assert.equal(page.properties.get("--baukit-auth-on-primary"), "#0F172A");

  const dark = await runTheme(`state=ap1.d.1E1B4B.312E81.${NONCE}`);
  assert.equal(dark.properties.get("--baukit-auth-on-primary"), "#FFFFFF");
});

test("the state survives a form post inside client_data", async () => {
  const page = await runTheme(
    `client_data=${clientData(`ap1.d.${NONCE}`)}&tab_id=x`,
  );

  assert.equal(page.root.dataset.baukitTheme, "dark");
});

test("unknown, short, or unreadable state changes nothing", async () => {
  for (const query of [
    "",
    "state=plain-random-state",
    `state=ap1.d.${NONCE.slice(2)}`,
    `state=ap1.x.${NONCE}`,
    `state=ap1.d.0A7CFF.${NONCE}`,
    "client_data=%%%",
  ]) {
    const page = await runTheme(query);
    assert.equal(page.root.dataset.baukitTheme, undefined, query);
    assert.equal(page.properties.size, 0, query);
    assert.equal(page.observers.length, 0, query);
  }
});
