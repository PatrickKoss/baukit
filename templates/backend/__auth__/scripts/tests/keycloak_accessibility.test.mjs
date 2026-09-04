import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const SCRIPT_URL = new URL(
  "../../keycloak/themes/baukit-accessible/login/resources/js/accessibility.js",
  import.meta.url,
);

class FakeElement {
  constructor(document, tagName, options = {}) {
    this.ownerDocument = document;
    this.tagName = tagName.toUpperCase();
    this.id = options.id ?? "";
    this.name = options.name ?? "";
    this.type = options.type ?? "";
    this.value = options.value ?? "";
    this.checked = options.checked ?? false;
    this.disabled = options.disabled ?? false;
    this.hidden = options.hidden ?? false;
    this.required = options.required ?? false;
    this.textContent = options.textContent ?? "";
    this.attributes = new Map();
    this.children = [];
    this.listeners = new Map();
    this.parentElement = null;
    if (options.className) this.setAttribute("class", options.className);
    if (this.id) document.register(this);
  }

  get validationMessage() {
    return this.required && this.value.length === 0
      ? "Fill out this field."
      : "";
  }

  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  append(child) {
    child.parentElement = this;
    this.children.push(child);
    this.ownerDocument.register(child);
  }

  closest(selector) {
    let current = this;
    while (current) {
      if (current.matches(selector)) return current;
      current = current.parentElement;
    }
    return null;
  }

  contains(candidate) {
    return (
      candidate === this ||
      this.children.some((child) => child.contains(candidate))
    );
  }

  dispatch(type, event = {}) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }

  focus() {
    this.ownerDocument.activeElement = this;
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  hasAttribute(name) {
    return this.attributes.has(name);
  }

  insertAdjacentElement(_position, child) {
    this.parentElement?.append(child);
  }

  matches(selector) {
    return selector.split(",").some((part) => {
      const candidate = part.trim();
      if (
        candidate === "input" ||
        candidate === "select" ||
        candidate === "textarea"
      ) {
        return this.tagName.toLowerCase() === candidate;
      }
      if (candidate.startsWith(".")) {
        const requiredClasses = candidate.split(".").filter(Boolean);
        const classes = (this.getAttribute("class") ?? "").split(/\s+/u);
        return requiredClasses.every((className) =>
          classes.includes(className),
        );
      }
      if (candidate === '[data-alert="danger"]') {
        return this.getAttribute("data-alert") === "danger";
      }
      return false;
    });
  }

  querySelector(selector) {
    return this.querySelectorAll(selector)[0] ?? null;
  }

  querySelectorAll(selector) {
    const matches = [];
    for (const child of this.children) {
      if (child.matches(selector)) matches.push(child);
      matches.push(...child.querySelectorAll(selector));
    }
    return matches;
  }

  remove() {
    if (this.parentElement) {
      this.parentElement.children = this.parentElement.children.filter(
        (child) => child !== this,
      );
    }
    this.ownerDocument.unregister(this);
    this.parentElement = null;
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }
}

class FakeDocument {
  constructor() {
    this.activeElement = null;
    this.elements = new Map();
    this.readyState = "complete";
    this.root = new FakeElement(this, "main");
  }

  addEventListener() {}

  createElement(tagName) {
    return new FakeElement(this, tagName);
  }

  getElementById(id) {
    return this.elements.get(id) ?? null;
  }

  querySelector(selector) {
    return this.root.querySelector(selector);
  }

  register(element) {
    if (element.id) this.elements.set(element.id, element);
  }

  unregister(element) {
    if (element.id) this.elements.delete(element.id);
  }
}

function append(parent, child) {
  parent.append(child);
  return child;
}

function form(document, id) {
  return append(document.root, new FakeElement(document, "form", { id }));
}

function field(document, parent, id, options = {}) {
  const group = append(
    parent,
    new FakeElement(document, "div", { className: "pf-v5-c-form__group" }),
  );
  if (options.requiredMarker) {
    append(
      group,
      new FakeElement(document, "span", {
        className: "pf-v5-c-form__label-required",
      }),
    );
  }
  const control = append(
    group,
    new FakeElement(document, "input", { id, name: id, ...options }),
  );
  append(
    group,
    new FakeElement(document, "div", { id: `input-error-container-${id}` }),
  );
  return control;
}

function submitEvent() {
  return {
    defaultPrevented: false,
    propagationStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopImmediatePropagation() {
      this.propagationStopped = true;
    },
  };
}

async function runTheme(document) {
  const source = await readFile(SCRIPT_URL, "utf8");
  vm.runInNewContext(source, { document });
}

test("empty login describes every error and focuses username", async () => {
  const document = new FakeDocument();
  const login = form(document, "kc-form-login");
  const username = field(document, login, "username");
  const password = field(document, login, "password", { type: "password" });
  username.setAttribute("aria-describedby", "username-help");

  await runTheme(document);
  const event = submitEvent();
  login.dispatch("submit", event);

  assert.equal(event.defaultPrevented, true);
  assert.equal(event.propagationStopped, true);
  assert.equal(username.required, true);
  assert.equal(password.required, true);
  assert.equal(username.getAttribute("aria-required"), "true");
  assert.equal(password.getAttribute("aria-invalid"), "true");
  assert.equal(
    username.getAttribute("aria-describedby"),
    "username-help baukit-client-error-username",
  );
  for (const id of [
    "baukit-client-error-username",
    "baukit-client-error-password",
  ]) {
    const error = document.getElementById(id);
    assert.equal(error.getAttribute("role"), "alert");
    assert.equal(error.getAttribute("aria-live"), "assertive");
    assert.equal(error.textContent, "Fill out this field.");
  }
  assert.equal(document.activeElement, username);
});

test("registration discovers only marked standard fields", async () => {
  const document = new FakeDocument();
  const registration = form(document, "kc-register-form");
  const required = [
    "username",
    "password",
    "password-confirm",
    "email",
    "firstName",
    "lastName",
  ].map((id) => field(document, registration, id, { requiredMarker: true }));
  const optional = field(document, registration, "nickname");

  await runTheme(document);
  registration.dispatch("submit", submitEvent());

  assert.equal(document.activeElement, required[0]);
  assert.equal(optional.required, false);
  for (const control of required) {
    assert.equal(control.required, true);
    const description = control.getAttribute("aria-describedby");
    assert.ok(description);
    for (const id of description.split(/\s+/u))
      assert.ok(document.getElementById(id));
  }
});

test("typing clears only the owned error and keeps other state", async () => {
  const document = new FakeDocument();
  const login = form(document, "kc-form-login");
  const username = field(document, login, "username");
  const password = field(document, login, "password", { type: "password" });
  username.setAttribute("aria-describedby", "username-help");
  login.dispatchCount = 0;

  await runTheme(document);
  login.dispatch("submit", submitEvent());
  username.value = "test";
  username.dispatch("input");

  assert.equal(document.getElementById("baukit-client-error-username"), null);
  assert.ok(document.getElementById("baukit-client-error-password"));
  assert.equal(username.getAttribute("aria-describedby"), "username-help");
  assert.equal(password.getAttribute("aria-invalid"), "true");

  password.value = "development-password";
  password.dispatch("input");
  const valid = submitEvent();
  login.dispatch("submit", valid);
  assert.equal(valid.defaultPrevented, false);
});

test("server field errors stay associated, live, and focused", async () => {
  const document = new FakeDocument();
  const login = form(document, "kc-form-login");
  const username = field(document, login, "username");
  field(document, login, "password", { type: "password" });
  username.setAttribute("aria-invalid", "true");
  username.setAttribute("aria-describedby", "username-help");
  const serverError = append(
    username.parentElement,
    new FakeElement(document, "span", {
      id: "input-error-username",
      textContent: "Invalid username or password.",
    }),
  );

  await runTheme(document);

  assert.equal(serverError.getAttribute("role"), "alert");
  assert.equal(serverError.getAttribute("aria-live"), "assertive");
  assert.equal(
    username.getAttribute("aria-describedby"),
    "username-help input-error-username",
  );
  assert.equal(document.activeElement, username);

  login.dispatch("submit", submitEvent());
  username.value = "test";
  username.dispatch("input");
  assert.equal(username.getAttribute("aria-invalid"), "true");
  assert.equal(
    username.getAttribute("aria-describedby"),
    "username-help input-error-username",
  );
});

test("a global credential alert receives focus when no field error exists", async () => {
  const document = new FakeDocument();
  const login = form(document, "kc-form-login");
  field(document, login, "username");
  field(document, login, "password", { type: "password" });
  const alert = append(
    document.root,
    new FakeElement(document, "div", {
      className: "pf-v5-c-alert pf-m-danger",
    }),
  );

  await runTheme(document);

  assert.equal(alert.getAttribute("role"), "alert");
  assert.equal(alert.getAttribute("aria-live"), "assertive");
  assert.equal(alert.getAttribute("tabindex"), "-1");
  assert.equal(document.activeElement, alert);
});

test("hidden username and repeated initialization do not alter the active flow", async () => {
  const document = new FakeDocument();
  const login = form(document, "kc-form-login");
  const username = field(document, login, "username", {
    hidden: true,
  });
  username.setAttribute("autocomplete", "username webauthn");
  const password = field(document, login, "password", { type: "password" });
  password.setAttribute("autocomplete", "current-password");

  await runTheme(document);
  await runTheme(document);
  assert.equal((login.listeners.get("submit") ?? []).length, 1);
  login.dispatch("submit", submitEvent());

  assert.equal(username.required, false);
  assert.equal(document.getElementById("baukit-client-error-username"), null);
  assert.ok(document.getElementById("baukit-client-error-password"));
  assert.equal(username.getAttribute("autocomplete"), "username webauthn");
  assert.equal(password.getAttribute("autocomplete"), "current-password");
});

test("native invalid events cover the inherited username-hidden form", async () => {
  const document = new FakeDocument();
  const login = form(document, "kc-form-login");
  const password = field(document, login, "password", { type: "password" });
  password.setAttribute("autocomplete", "current-password");

  await runTheme(document);
  const invalid = submitEvent();
  invalid.target = password;
  login.dispatch("invalid", invalid);
  await Promise.resolve();

  assert.equal(invalid.defaultPrevented, true);
  assert.ok(document.getElementById("baukit-client-error-password"));
  assert.equal(document.activeElement, password);
  assert.equal(password.getAttribute("autocomplete"), "current-password");
});

test("other Keycloak pages remain unchanged", async () => {
  const document = new FakeDocument();
  const reset = form(document, "kc-reset-password-form");
  const username = field(document, reset, "username", { requiredMarker: true });

  await runTheme(document);

  assert.equal(username.required, false);
  assert.equal(username.attributes.size, 0);
});
