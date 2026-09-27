// Checks media-grant.js against fixtures/media-grants/vectors-v1.json. Runs in
// Node and in both njs engines, so it follows the same syntax limits.

function describe(value) {
  return JSON.stringify(value);
}

function outcome(action) {
  try {
    return { value: action() };
  } catch (error) {
    return { error: error.code || String(error) };
  }
}

function mismatch(section, name, expected, actual) {
  return `${section} ${name}: expected ${describe(expected)}, got ${describe(actual)}`;
}

function loadKeys(corpus, grants) {
  const keys = {};
  corpus.keys.forEach((fixture) => {
    keys[fixture.name] = grants.loadMediaGrantKey(
      fixture.keyId,
      fixture.secretBase64url,
    );
  });
  return keys;
}

function ringFor(keys, grants, current, previous) {
  return grants.loadMediaGrantKeyRing(
    keys[current],
    previous === null ? null : keys[previous],
  );
}

function checkProtocol(corpus, grants) {
  const expected = describe(corpus.protocol);
  const actual = describe(grants.protocol);
  return expected === actual ? [] : [mismatch("protocol", "constants", corpus.protocol, grants.protocol)];
}

function checkKeyCases(corpus, grants) {
  const failures = [];
  corpus.keyCases.forEach((testCase) => {
    const result = outcome(() =>
      grants.loadMediaGrantKey(testCase.keyId, testCase.secretBase64url)
    );
    const actual = result.error === undefined
      ? { keyId: result.value.keyId }
      : { error: result.error };
    if (describe(actual) !== describe(testCase.expected)) {
      failures.push(mismatch("keyCases", testCase.name, testCase.expected, actual));
    }
  });
  return failures;
}

function checkRingCases(corpus, grants, keys) {
  const failures = [];
  corpus.ringCases.forEach((testCase) => {
    const result = outcome(() =>
      ringFor(keys, grants, testCase.current, testCase.previous)
    );
    const ring = result.value;
    const actual = result.error === undefined
      ? {
        keyIds: [ring.current.keyId].concat(
          ring.previous === null ? [] : [ring.previous.keyId],
        ),
      }
      : { error: result.error };
    if (describe(actual) !== describe(testCase.expected)) {
      failures.push(mismatch("ringCases", testCase.name, testCase.expected, actual));
    }
  });
  return failures;
}

// The edge only verifies: each signed case must verify at its own clock, and
// signing refusals are checked against the Rust signer only.
async function checkSignCases(corpus, grants, keys) {
  const failures = [];
  const signed = corpus.signCases.filter((testCase) =>
    testCase.expected.error === undefined
  );
  for (let index = 0; index < signed.length; index++) {
    const testCase = signed[index];
    const key = keys[testCase.key];
    const input = grants.signingInput(testCase.path, testCase.expires, key.keyId);
    if (input !== testCase.expected.signingInput) {
      failures.push(mismatch("signCases", testCase.name, testCase.expected.signingInput, input));
    }
    const result = await grants.verifyMediaGrant(
      grants.loadMediaGrantKeyRing(key, null),
      {
        method: "GET",
        path: testCase.path,
        query: testCase.expected.query,
        now: testCase.now,
      },
    );
    const expected = { ok: true, keyId: key.keyId, expires: testCase.expires };
    if (describe(result) !== describe(expected)) {
      failures.push(mismatch("signCases", testCase.name, expected, result));
    }
  }
  return failures;
}

async function checkVerifyCases(corpus, grants, keys) {
  const failures = [];
  for (let index = 0; index < corpus.verifyCases.length; index++) {
    const testCase = corpus.verifyCases[index];
    const result = await grants.verifyMediaGrant(
      ringFor(keys, grants, testCase.current, testCase.previous),
      {
        method: testCase.method,
        path: testCase.path,
        query: testCase.query,
        now: testCase.now,
      },
    );
    const actual = result.ok
      ? { keyId: result.keyId, expires: result.expires }
      : { error: result.error };
    if (describe(actual) !== describe(testCase.expected)) {
      failures.push(mismatch("verifyCases", testCase.name, testCase.expected, actual));
    }
  }
  return failures;
}

async function checkVectors(corpus, grants) {
  const keys = loadKeys(corpus, grants);
  const signFailures = await checkSignCases(corpus, grants, keys);
  const verifyFailures = await checkVerifyCases(corpus, grants, keys);
  return {
    cases: corpus.keyCases.length + corpus.ringCases.length +
      corpus.signCases.length + corpus.verifyCases.length,
    failures: checkProtocol(corpus, grants)
      .concat(checkKeyCases(corpus, grants))
      .concat(checkRingCases(corpus, grants, keys))
      .concat(signFailures)
      .concat(verifyFailures),
  };
}

export default {
  checkKeyCases: checkKeyCases,
  checkProtocol: checkProtocol,
  checkRingCases: checkRingCases,
  checkSignCases: checkSignCases,
  checkVectors: checkVectors,
  checkVerifyCases: checkVerifyCases,
  loadKeys: loadKeys,
};
