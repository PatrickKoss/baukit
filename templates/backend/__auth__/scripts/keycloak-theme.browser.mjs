import assert from "node:assert/strict";
import { createRequire } from "node:module";

const requireFromWeb = createRequire(
  new URL("../web/package.json", import.meta.url),
);
const { chromium } = requireFromWeb("@playwright/test");

const baseUrl = requiredEnvironment("KEYCLOAK_BASE_URL").replace(/\/$/u, "");
const realmName = requiredEnvironment("KEYCLOAK_REALM");
const clientId = requiredEnvironment("KEYCLOAK_CLIENT_ID");
const redirectUri = requiredEnvironment("KEYCLOAK_REDIRECT_URI");
const adminUsername = requiredEnvironment("KEYCLOAK_ADMIN_USERNAME");
const adminPassword = requiredEnvironment("KEYCLOAK_ADMIN_PASSWORD");
const testUsername = requiredEnvironment("KEYCLOAK_TEST_USERNAME");
const testPassword = requiredEnvironment("KEYCLOAK_TEST_PASSWORD");

function requiredEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return value;
}

async function adminRequest(path, options = {}) {
  const response = await fetch(`${baseUrl}${path}`, {
    ...options,
    headers: {
      authorization: `Bearer ${adminRequest.token}`,
      "content-type": "application/json",
      ...options.headers,
    },
  });
  if (!response.ok)
    throw new Error(
      `Keycloak Admin API ${options.method ?? "GET"} ${path} failed`,
    );
  if (response.status === 201 || response.status === 204) return null;
  return response.json();
}

adminRequest.token = "";

async function authenticateAdministrator() {
  const body = new URLSearchParams({
    client_id: "admin-cli",
    grant_type: "password",
    username: adminUsername,
    password: adminPassword,
  });
  const response = await fetch(
    `${baseUrl}/realms/master/protocol/openid-connect/token`,
    {
      method: "POST",
      body,
    },
  );
  if (!response.ok)
    throw new Error("Keycloak administrator authentication failed");
  adminRequest.token = (await response.json()).access_token;
}

async function realm() {
  return adminRequest(`/admin/realms/${encodeURIComponent(realmName)}`);
}

async function updateRealm(changes) {
  const current = await realm();
  await adminRequest(`/admin/realms/${encodeURIComponent(realmName)}`, {
    method: "PUT",
    body: JSON.stringify({ ...current, ...changes }),
  });
}

function authorizationUrl(registration = false, locale = null) {
  const endpoint = registration ? "registrations" : "auth";
  const url = new URL(
    `${baseUrl}/realms/${encodeURIComponent(realmName)}/protocol/openid-connect/${endpoint}`,
  );
  url.searchParams.set("client_id", clientId);
  url.searchParams.set("redirect_uri", redirectUri);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("scope", "openid");
  url.searchParams.set(
    "code_challenge",
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  );
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("prompt", "login");
  if (locale) url.searchParams.set("kc_locale", locale);
  return url.href;
}

async function openLogin(page) {
  await page.goto(authorizationUrl());
  await page.locator("#kc-form-login").waitFor();
  assert.equal(
    await page
      .locator("#kc-form-login")
      .getAttribute("data-baukit-accessibility-initialized"),
    "true",
  );
}

async function describedElementsExist(page, selector) {
  return page.locator(selector).evaluateAll((controls) =>
    controls.every((control) =>
      (control.getAttribute("aria-describedby") ?? "")
        .split(/\s+/u)
        .filter(Boolean)
        .every((id) => document.getElementById(id) !== null),
    ),
  );
}

async function testEmptyLogin(page) {
  await page.context().clearCookies();
  await openLogin(page);
  await page.locator("#kc-login").click();
  const username = page.locator("#username");
  const password = page.locator("#password");
  for (const control of [username, password]) {
    await assert.doesNotReject(() => control.waitFor());
    assert.equal(await control.getAttribute("required"), "");
    assert.equal(await control.getAttribute("aria-required"), "true");
    assert.equal(await control.getAttribute("aria-invalid"), "true");
  }
  assert.equal(await page.locator("[data-baukit-client-error]").count(), 2);
  assert.equal(
    await describedElementsExist(page, "#username, #password"),
    true,
  );
  for (const error of await page.locator("[data-baukit-client-error]").all()) {
    assert.equal(await error.getAttribute("role"), "alert");
    assert.equal(await error.getAttribute("aria-live"), "assertive");
    assert.notEqual((await error.textContent())?.trim(), "");
  }
  assert.equal(
    await page.evaluate(() => document.activeElement?.id),
    "username",
  );
  console.log("PASS empty login");
}

async function testRecoveryAndValidPost(page) {
  await page.context().clearCookies();
  await openLogin(page);
  await page.locator("#username").evaluate((control) => {
    const help = document.createElement("span");
    help.id = "pre-existing-help";
    document.body.append(help);
    control.setAttribute("aria-describedby", "pre-existing-help");
  });
  await page.locator("#kc-login").click();
  await page.locator("#username").fill(testUsername);
  assert.equal(await page.locator("#baukit-client-error-username").count(), 0);
  assert.equal(await page.locator("#baukit-client-error-password").count(), 1);
  assert.match(
    await page.locator("#username").getAttribute("aria-describedby"),
    /pre-existing-help/u,
  );

  let posts = 0;
  page.on("request", (request) => {
    if (
      request.method() === "POST" &&
      request.url().includes("/login-actions/authenticate")
    )
      posts += 1;
  });
  await page.route(`${redirectUri}**`, (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/html",
      body: "<title>redirect target</title>",
    }),
  );
  await page.locator("#password").fill(testPassword);
  const post = page.waitForRequest(
    (request) =>
      request.method() === "POST" &&
      request.url().includes("/login-actions/authenticate"),
  );
  await page.locator("#kc-login").click({ noWaitAfter: true });
  await post;
  assert.equal(posts, 1);
  console.log("PASS recovery and valid post");
}

async function testServerCredentialError(page) {
  await page.context().clearCookies();
  await openLogin(page);
  await page.locator("#username").fill(testUsername);
  await page.locator("#password").fill(`${testPassword}-wrong`);
  await page.locator("#kc-login").click();
  await page.locator("#kc-form-login").waitFor();

  const fieldError = page.locator('[id^="input-error-"][role="alert"]');
  if ((await fieldError.count()) > 0) {
    const controlId = (await fieldError.first().getAttribute("id")).replace(
      /^input-error-/u,
      "",
    );
    const control = page.locator(`#${controlId}`);
    assert.equal(await control.getAttribute("aria-invalid"), "true");
    assert.match(
      await control.getAttribute("aria-describedby"),
      new RegExp(`input-error-${controlId}`, "u"),
    );
    assert.equal(
      await page.evaluate(() => document.activeElement?.id),
      controlId,
    );
    await control.fill("");
    await page.locator("#kc-login").click();
    await control.fill(testUsername);
    assert.equal(await fieldError.first().count(), 1);
    assert.match(
      await control.getAttribute("aria-describedby"),
      new RegExp(`input-error-${controlId}`, "u"),
    );
    assert.equal(await control.getAttribute("aria-invalid"), "true");
    assert.equal(
      await page.locator("#baukit-client-error-password").count(),
      1,
    );
  } else {
    const alert = page.locator('[role="alert"][aria-live="assertive"]').first();
    await alert.waitFor();
    assert.equal(await alert.getAttribute("tabindex"), "-1");
    assert.equal(
      await alert.evaluate((element) => document.activeElement === element),
      true,
    );
  }
  console.log("PASS server credential error");
}

async function testEmptyRegistration(page) {
  await updateRealm({ registrationAllowed: true });
  await page.context().clearCookies();
  await page.goto(authorizationUrl(true));
  const registration = page.locator("#kc-register-form");
  await registration.waitFor();
  const requiredControls = registration.locator(
    '[required][aria-required="true"]',
  );
  const count = await requiredControls.count();
  assert.equal(count, 6);
  await registration.locator('[type="submit"]').click();
  assert.equal(await page.locator("[data-baukit-client-error]").count(), count);
  assert.equal(
    await describedElementsExist(page, "#kc-register-form [required]"),
    true,
  );
  assert.equal(
    await page.evaluate(() => document.activeElement?.id),
    "username",
  );
  for (const id of ["password", "password-confirm"]) {
    assert.equal(
      await page.locator(`#${id}`).getAttribute("aria-invalid"),
      "true",
    );
  }
  console.log("PASS empty registration");
}

async function testConditionalWebAuthn(page) {
  await updateRealm({ webAuthnPolicyPasswordlessPasskeysEnabled: true });
  await page.context().clearCookies();
  await openLogin(page);
  const username = page.locator("#username");
  assert.equal(
    await username.getAttribute("autocomplete"),
    "username webauthn",
  );
  await page.locator("#kc-login").click();
  assert.equal(await page.locator("[data-baukit-client-error]").count(), 2);
  assert.equal(
    await username.getAttribute("autocomplete"),
    "username webauthn",
  );
  console.log("PASS conditional WebAuthn login");
}

async function createSplitBrowserFlow() {
  const alias = `baukit-split-browser-${Date.now()}`;
  const response = await adminRequest(
    `/admin/realms/${encodeURIComponent(realmName)}/authentication/flows`,
    {
      method: "POST",
      body: JSON.stringify({
        alias,
        providerId: "basic-flow",
        topLevel: true,
        builtIn: false,
      }),
    },
  );
  assert.equal(response, null);
  const flows = await adminRequest(
    `/admin/realms/${encodeURIComponent(realmName)}/authentication/flows`,
  );
  const flow = flows.find((candidate) => candidate.alias === alias);
  assert.ok(flow);
  for (const [provider, priority] of [
    ["auth-username-form", 10],
    ["auth-password-form", 20],
  ]) {
    await adminRequest(
      `/admin/realms/${encodeURIComponent(realmName)}/authentication/flows/${encodeURIComponent(alias)}/executions/execution`,
      {
        method: "POST",
        body: JSON.stringify({ provider, requirement: "REQUIRED", priority }),
      },
    );
    const executions = await adminRequest(
      `/admin/realms/${encodeURIComponent(realmName)}/authentication/flows/${encodeURIComponent(alias)}/executions`,
    );
    const execution = executions.find(
      (candidate) => candidate.providerId === provider,
    );
    assert.ok(execution);
    if (execution.requirement !== "REQUIRED") {
      await adminRequest(
        `/admin/realms/${encodeURIComponent(realmName)}/authentication/flows/${encodeURIComponent(alias)}/executions`,
        {
          method: "PUT",
          body: JSON.stringify({
            id: execution.id,
            requirement: "REQUIRED",
            priority,
          }),
        },
      );
    }
  }
  return flow;
}

async function testUsernameHidden(page, originalBrowserFlow) {
  const flow = await createSplitBrowserFlow();
  try {
    await updateRealm({ browserFlow: flow.alias });
    await page.context().clearCookies();
    await openLogin(page);
    assert.equal(await page.locator("#username").count(), 1);
    assert.equal(await page.locator("#password").count(), 0);
    await page.locator("#username").fill(testUsername);
    await page.locator("#kc-login").click();
    await page.locator("#password").waitFor();
    assert.equal(await page.locator("#username").count(), 0);
    assert.equal(
      await page.locator("#password").getAttribute("autocomplete"),
      "current-password",
    );
    await page.locator("#kc-login").click();
    assert.equal(await page.locator("[data-baukit-client-error]").count(), 1);
    assert.equal(
      await page.evaluate(() => document.activeElement?.id),
      "password",
    );
  } finally {
    await updateRealm({ browserFlow: originalBrowserFlow });
    await adminRequest(
      `/admin/realms/${encodeURIComponent(realmName)}/authentication/flows/${encodeURIComponent(flow.id)}`,
      { method: "DELETE" },
    );
  }
  console.log("PASS username-hidden login");
}

async function testChildOverlay(page) {
  await updateRealm({
    loginTheme: "baukit-accessible-test",
    internationalizationEnabled: true,
    supportedLocales: ["en", "de"],
    defaultLocale: "en",
  });
  await page.context().clearCookies();
  await page.goto(authorizationUrl(false, "en"));
  await page.locator("#kc-form-login").waitFor();
  await assert.doesNotReject(() =>
    page.getByRole("heading", { name: "Neutral overlay sign in" }).waitFor(),
  );
  assert.equal(
    await page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue("--baukit-accessible-test-overlay")
        .trim(),
    ),
    "loaded",
  );
  await page.locator("#kc-login").click();
  assert.equal(await page.locator("[data-baukit-client-error]").count(), 2);
  console.log("PASS child overlay with internationalization");
}

await authenticateAdministrator();
const originalRealm = await realm();
assert.equal(originalRealm.loginTheme, "baukit-accessible");
const browser = await chromium.launch();
try {
  for (const browserTest of [
    testEmptyLogin,
    testRecoveryAndValidPost,
    testServerCredentialError,
    testEmptyRegistration,
    testConditionalWebAuthn,
    (page) => testUsernameHidden(page, originalRealm.browserFlow),
    testChildOverlay,
  ]) {
    const page = await browser.newPage();
    try {
      await browserTest(page);
    } finally {
      await page.close();
    }
  }
} finally {
  await updateRealm({
    browserFlow: originalRealm.browserFlow,
    defaultLocale: originalRealm.defaultLocale,
    internationalizationEnabled: originalRealm.internationalizationEnabled,
    loginTheme: originalRealm.loginTheme,
    registrationAllowed: originalRealm.registrationAllowed,
    supportedLocales: originalRealm.supportedLocales,
    webAuthnPolicyPasswordlessPasskeysEnabled:
      originalRealm.webAuthnPolicyPasswordlessPasskeysEnabled,
  });
  await browser.close();
}
