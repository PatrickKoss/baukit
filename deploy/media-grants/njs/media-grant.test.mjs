import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, mock, test } from "node:test";

import grants from "./media-grant.js";
import checks from "./vector-checks.js";

const corpus = JSON.parse(
  readFileSync(
    new URL("../../../fixtures/media-grants/vectors-v1.json", import.meta.url),
    "utf8",
  ),
);
const MILLISECONDS_PER_SECOND = 1000;
const keyFixture = (name) => corpus.keys.find((key) => key.name === name);
const signed = corpus.signCases.find((testCase) => testCase.name === "sign-current");
const signedByPrevious = corpus.signCases.find(
  (testCase) => testCase.name === "sign-previous",
);
const ENV_NAMES = [
  "MEDIA_GRANT_KEY_ID",
  "MEDIA_GRANT_SIGNING_KEY",
  "MEDIA_GRANT_PREVIOUS_KEY_ID",
  "MEDIA_GRANT_PREVIOUS_SIGNING_KEY",
];

describe("shared vectors", () => {
  const keys = checks.loadKeys(corpus, grants);

  test("protocol constants match", () => {
    assert.deepEqual(checks.checkProtocol(corpus, grants), []);
  });

  test("key cases load or fail with the expected code", () => {
    assert.ok(corpus.keyCases.length > 0);
    assert.deepEqual(checks.checkKeyCases(corpus, grants), []);
  });

  test("ring cases build or fail with the expected code", () => {
    assert.ok(corpus.ringCases.length > 0);
    assert.deepEqual(checks.checkRingCases(corpus, grants, keys), []);
  });

  test("independently signed grants verify", async () => {
    assert.ok(corpus.signCases.length > 0);
    assert.deepEqual(await checks.checkSignCases(corpus, grants, keys), []);
  });

  test("verify cases accept or reject with the expected code", async () => {
    assert.ok(corpus.verifyCases.length > 0);
    assert.deepEqual(await checks.checkVerifyCases(corpus, grants, keys), []);
  });
});

describe("keys", () => {
  test("serializing a key never shows its secret", () => {
    const fixture = keyFixture("current");
    const key = grants.loadMediaGrantKey(fixture.keyId, fixture.secretBase64url);
    const text = `${JSON.stringify(key)} ${String(key)}`;
    assert.ok(!text.includes(fixture.secretBase64url));
    assert.ok(JSON.stringify(key).includes(fixture.keyId));
  });

  test("a previous key needs both an ID and a secret", () => {
    const current = keyFixture("current");
    const env = {
      MEDIA_GRANT_KEY_ID: current.keyId,
      MEDIA_GRANT_SIGNING_KEY: current.secretBase64url,
      MEDIA_GRANT_PREVIOUS_KEY_ID: "retired",
    };
    assert.throws(() => grants.loadMediaGrantKeyRingFromEnv(env), {
      code: "invalid_secret_encoding",
    });
    assert.equal(
      grants.loadMediaGrantKeyRingFromEnv({
        MEDIA_GRANT_KEY_ID: current.keyId,
        MEDIA_GRANT_SIGNING_KEY: current.secretBase64url,
      }).previous,
      null,
    );
  });
});

function targetVariables(target, method = "GET") {
  return { request: `${method} ${target} HTTP/1.1`, request_uri: target };
}

function fakeRequest(overrides) {
  const request = {
    method: "GET",
    uri: signed.path,
    status: 200,
    variables: {
      ...targetVariables(`${signed.path}?${signed.expected.query}`),
      media_grant_cache_control: "no-store",
    },
    headersOut: {},
    returned: null,
    errors: [],
    return(status) {
      this.returned = status;
    },
    error(message) {
      this.errors.push(message);
    },
  };
  return Object.assign(request, overrides);
}

describe("nginx adapter", () => {
  let savedEnv;

  beforeEach(() => {
    savedEnv = Object.fromEntries(ENV_NAMES.map((name) => [name, process.env[name]]));
    const current = keyFixture("current");
    const previous = keyFixture("previous");
    process.env.MEDIA_GRANT_KEY_ID = current.keyId;
    process.env.MEDIA_GRANT_SIGNING_KEY = current.secretBase64url;
    process.env.MEDIA_GRANT_PREVIOUS_KEY_ID = previous.keyId;
    process.env.MEDIA_GRANT_PREVIOUS_SIGNING_KEY = previous.secretBase64url;
    mock.timers.enable({ apis: ["Date"], now: signed.now * MILLISECONDS_PER_SECOND });
  });

  afterEach(() => {
    mock.timers.reset();
    for (const name of ENV_NAMES) {
      if (savedEnv[name] === undefined) delete process.env[name];
      else process.env[name] = savedEnv[name];
    }
  });

  test("a valid grant passes and sets a private cache lifetime", async () => {
    const r = fakeRequest({});
    await grants.authorize(r);
    assert.equal(r.returned, null);
    assert.equal(
      r.variables.media_grant_cache_control,
      `private, max-age=${signed.expires - signed.now}, must-revalidate`,
    );
  });

  test("a grant from the previous key passes during rotation", async () => {
    const r = fakeRequest({
      uri: signedByPrevious.path,
      variables: targetVariables(
        `${signedByPrevious.path}?${signedByPrevious.expected.query}`,
      ),
    });
    await grants.authorize(r);
    assert.equal(r.returned, null);
  });

  test("a raw path that nginx decoded differently is refused", async () => {
    const encoded = signed.path.replace("/media/", "/%6Dedia/");
    const r = fakeRequest({
      variables: targetVariables(`${encoded}?${signed.expected.query}`),
    });
    await grants.authorize(r);
    assert.equal(r.returned, 403);
  });

  test("a request without a query is refused", async () => {
    const r = fakeRequest({ variables: targetVariables(signed.path) });
    await grants.authorize(r);
    assert.equal(r.returned, 403);
  });

  test("an absolute-form request target is refused", async () => {
    const target = `${signed.path}?${signed.expected.query}`;
    const r = fakeRequest({
      variables: {
        request: `GET http://media.example${target} HTTP/1.1`,
        request_uri: target,
      },
    });
    await grants.authorize(r);
    assert.equal(r.returned, 403);
  });

  test("an HTTP/2 request line passes", async () => {
    const target = `${signed.path}?${signed.expected.query}`;
    const r = fakeRequest({
      variables: { request: `GET ${target} HTTP/2.0`, request_uri: target },
    });
    await grants.authorize(r);
    assert.equal(r.returned, null);
  });

  test("a request line without an HTTP version is refused", async () => {
    const target = `${signed.path}?${signed.expected.query}`;
    const r = fakeRequest({
      variables: { request: `GET ${target}`, request_uri: target },
    });
    await grants.authorize(r);
    assert.equal(r.returned, 403);
  });

  test("a POST is refused", async () => {
    const r = fakeRequest({
      method: "POST",
      variables: targetVariables(`${signed.path}?${signed.expected.query}`, "POST"),
    });
    await grants.authorize(r);
    assert.equal(r.returned, 403);
  });

  test("an expired grant is refused", async () => {
    mock.timers.setTime(signed.expires * MILLISECONDS_PER_SECOND);
    const r = fakeRequest({});
    await grants.authorize(r);
    assert.equal(r.returned, 403);
  });

  test("broken key configuration is refused and logs only the code", async () => {
    process.env.MEDIA_GRANT_SIGNING_KEY = "c2hvcnQ";
    const r = fakeRequest({});
    await grants.authorize(r);
    assert.equal(r.returned, 403);
    assert.deepEqual(r.errors, ["media grant keys are invalid: secret_too_short"]);
  });

  test("success responses get the grant cache lifetime", () => {
    const r = fakeRequest({
      variables: { media_grant_cache_control: "private, max-age=10, must-revalidate" },
    });
    grants.responseHeaders(r);
    assert.equal(r.headersOut["Cache-Control"], "private, max-age=10, must-revalidate");
  });

  test("error responses drop the grant cache lifetime", () => {
    const r = fakeRequest({
      status: 416,
      headersOut: { "Cache-Control": "private, max-age=10, must-revalidate" },
    });
    grants.responseHeaders(r);
    assert.equal(r.headersOut["Cache-Control"], undefined);
  });
});
