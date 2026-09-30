// njs 1.0.1 (native engine and QuickJS) and Node 24. The native engine has no
// destructuring, for...of, classes, or named exports, so this file uses none.

const MEDIA_GRANT_MODE = "playback";
const MAX_GRANT_LIFETIME_SECONDS = 3600;
const MAX_CLOCK_SKEW_SECONDS = 60;
const MIN_SECRET_BYTES = 32;
const MAX_KEY_ID_BYTES = 64;
const MAX_PATH_BYTES = 512;
const MAX_EXPIRES_DIGITS = 10;
const SIGNATURE_BYTES = 32;
const SIGNATURE_BASE64URL_LENGTH = 43;
const QUERY_NAMES = ["expires", "keyId", "mode", "signature"];
const QUERY_SEPARATOR_BYTES = QUERY_NAMES.length * 2 - 1;
const MAX_QUERY_BYTES = QUERY_NAMES.join("").length + QUERY_SEPARATOR_BYTES +
  MAX_EXPIRES_DIGITS + MAX_KEY_ID_BYTES + MEDIA_GRANT_MODE.length +
  SIGNATURE_BASE64URL_LENGTH;
const METHODS = ["GET", "HEAD"];
const KEY_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;
const BASE64URL_PATTERN = /^[A-Za-z0-9_-]+$/;
const EXPIRES_PATTERN = /^[1-9][0-9]*$/;
const PATH_SEGMENT_PATTERN = /^[A-Za-z0-9_-][A-Za-z0-9._-]*$/;
const KEY_ENV = {
  currentKeyId: "MEDIA_GRANT_KEY_ID",
  currentSecret: "MEDIA_GRANT_SIGNING_KEY",
  previousKeyId: "MEDIA_GRANT_PREVIOUS_KEY_ID",
  previousSecret: "MEDIA_GRANT_PREVIOUS_SIGNING_KEY",
};
const CACHE_CONTROL_VARIABLE = "media_grant_cache_control";
const FORBIDDEN = 403;
const FIRST_ERROR_STATUS = 400;
const MILLISECONDS_PER_SECOND = 1000;
const HTTP_VERSION_SEPARATOR = " HTTP/";

function configurationError(code) {
  const error = new Error(code);
  error.code = code;
  return error;
}

function rejected(code) {
  return { ok: false, error: code };
}

function decodeCanonical(text) {
  if (typeof text !== "string" || !BASE64URL_PATTERN.test(text)) return null;
  const bytes = Buffer.from(text, "base64url");
  return bytes.toString("base64url") === text ? bytes : null;
}

function validKeyId(keyId) {
  return typeof keyId === "string" && keyId.length <= MAX_KEY_ID_BYTES &&
    KEY_ID_PATTERN.test(keyId);
}

async function hmacSha256(secret, input) {
  const key = await crypto.subtle.importKey(
    "raw",
    secret,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const tag = await crypto.subtle.sign("HMAC", key, Buffer.from(input, "utf8"));
  return new Uint8Array(tag);
}

function constantTimeEqual(left, right) {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index++) {
    difference |= left[index] ^ right[index];
  }
  return difference === 0;
}

function signingInput(path, expires, keyId) {
  return `${path}\n${expires}\n${MEDIA_GRANT_MODE}\n${keyId}`;
}

function validMediaPath(path) {
  if (typeof path !== "string" || path.charAt(0) !== "/") return false;
  if (Buffer.byteLength(path, "utf8") > MAX_PATH_BYTES) return false;
  return path.slice(1).split("/").every((segment) =>
    PATH_SEGMENT_PATTERN.test(segment)
  );
}

// The secret stays in a closure so JSON.stringify and logging never see it.
function loadMediaGrantKey(keyId, secretBase64url) {
  if (!validKeyId(keyId)) throw configurationError("invalid_key_id");
  const secret = decodeCanonical(secretBase64url);
  if (secret === null) throw configurationError("invalid_secret_encoding");
  if (secret.length < MIN_SECRET_BYTES) {
    throw configurationError("secret_too_short");
  }
  return Object.freeze({
    keyId: keyId,
    mac: (input) => hmacSha256(secret, input),
  });
}

function loadMediaGrantKeyRing(current, previous) {
  const retiring = previous || null;
  if (retiring !== null && retiring.keyId === current.keyId) {
    throw configurationError("duplicate_key_id");
  }
  return Object.freeze({ current: current, previous: retiring });
}

function loadMediaGrantKeyRingFromEnv(env) {
  const current = loadMediaGrantKey(
    env[KEY_ENV.currentKeyId],
    env[KEY_ENV.currentSecret],
  );
  const previousKeyId = env[KEY_ENV.previousKeyId] || "";
  const previousSecret = env[KEY_ENV.previousSecret] || "";
  const previous = previousKeyId === "" && previousSecret === ""
    ? null
    : loadMediaGrantKey(previousKeyId, previousSecret);
  return loadMediaGrantKeyRing(current, previous);
}

function parseQuery(query) {
  if (typeof query !== "string") return null;
  if (Buffer.byteLength(query, "utf8") > MAX_QUERY_BYTES) return null;
  const parts = query.split("&");
  if (parts.length !== QUERY_NAMES.length) return null;
  const values = [];
  for (let index = 0; index < parts.length; index++) {
    const prefix = `${QUERY_NAMES[index]}=`;
    const value = parts[index].slice(prefix.length);
    if (!parts[index].startsWith(prefix) || value === "") return null;
    values.push(value);
  }
  const expires = values[0];
  const keyId = values[1];
  const signature = values[3];
  if (values[2] !== MEDIA_GRANT_MODE || !validKeyId(keyId)) return null;
  if (expires.length > MAX_EXPIRES_DIGITS || !EXPIRES_PATTERN.test(expires)) {
    return null;
  }
  if (signature.length !== SIGNATURE_BASE64URL_LENGTH) return null;
  const signatureBytes = decodeCanonical(signature);
  if (signatureBytes === null || signatureBytes.length !== SIGNATURE_BYTES) {
    return null;
  }
  return {
    expires: Number(expires),
    keyId: keyId,
    signature: signatureBytes,
  };
}

function expiryError(expires, now) {
  if (expires <= now) return "expired";
  if (expires - now > MAX_GRANT_LIFETIME_SECONDS + MAX_CLOCK_SKEW_SECONDS) {
    return "expiry_too_far";
  }
  return null;
}

function selectKey(ring, keyId) {
  if (ring.current.keyId === keyId) return ring.current;
  if (ring.previous !== null && ring.previous.keyId === keyId) {
    return ring.previous;
  }
  return null;
}

// request: { method, path (raw, before decoding), query, now (Unix seconds) }.
async function verifyMediaGrant(ring, request) {
  if (METHODS.indexOf(request.method) === -1) return rejected("invalid_method");
  if (!validMediaPath(request.path)) return rejected("invalid_path");
  const grant = parseQuery(request.query);
  if (grant === null || !Number.isSafeInteger(request.now)) {
    return rejected("invalid_query");
  }
  const expiry = expiryError(grant.expires, request.now);
  if (expiry !== null) return rejected(expiry);
  const key = selectKey(ring, grant.keyId);
  if (key === null) return rejected("unknown_key");
  const expected = await key.mac(
    signingInput(request.path, grant.expires, key.keyId),
  );
  if (!constantTimeEqual(expected, grant.signature)) {
    return rejected("invalid_signature");
  }
  return { ok: true, keyId: key.keyId, expires: grant.expires };
}

// The raw target from the request line. nginx drops the scheme and host of an
// absolute-form target from $request_uri, so only the line shows them.
function requestTarget(r) {
  const line = r.variables.request;
  const prefix = `${r.method} `;
  if (typeof line !== "string" || !line.startsWith(prefix)) return null;
  const versionStart = line.indexOf(HTTP_VERSION_SEPARATOR, prefix.length);
  if (versionStart === -1) return null;
  return line.slice(prefix.length, versionStart);
}

function splitRequestUri(requestUri) {
  const queryStart = typeof requestUri === "string"
    ? requestUri.indexOf("?")
    : -1;
  if (queryStart <= 0) return null;
  return {
    path: requestUri.slice(0, queryStart),
    query: requestUri.slice(queryStart + 1),
  };
}

function nowSeconds() {
  return Math.floor(Date.now() / MILLISECONDS_PER_SECOND);
}

// nginx js_access handler. Keys come from process.env, never from nginx
// variables, so no log_format can print them.
async function authorize(r) {
  try {
    const rawTarget = requestTarget(r);
    if (rawTarget === null || rawTarget !== r.variables.request_uri) {
      r.return(FORBIDDEN);
      return;
    }
    const target = splitRequestUri(rawTarget);
    if (target === null || target.path !== r.uri) {
      r.return(FORBIDDEN);
      return;
    }
    const now = nowSeconds();
    const result = await verifyMediaGrant(
      loadMediaGrantKeyRingFromEnv(process.env),
      { method: r.method, path: target.path, query: target.query, now: now },
    );
    if (!result.ok) {
      r.return(FORBIDDEN);
      return;
    }
    r.variables[CACHE_CONTROL_VARIABLE] =
      `private, max-age=${result.expires - now}, must-revalidate`;
  } catch (error) {
    if (error.code) r.error(`media grant keys are invalid: ${error.code}`);
    r.return(FORBIDDEN);
  }
}

// nginx js_header_filter handler. It may run again after nginx turns a
// response into a 416, so error responses drop the grant's Cache-Control.
function responseHeaders(r) {
  if (r.status >= FIRST_ERROR_STATUS) {
    delete r.headersOut["Cache-Control"];
    return;
  }
  r.headersOut["Cache-Control"] = r.variables[CACHE_CONTROL_VARIABLE];
}

const protocol = Object.freeze({
  mode: MEDIA_GRANT_MODE,
  queryOrder: QUERY_NAMES,
  maxLifetimeSeconds: MAX_GRANT_LIFETIME_SECONDS,
  maxClockSkewSeconds: MAX_CLOCK_SKEW_SECONDS,
  minSecretBytes: MIN_SECRET_BYTES,
  maxKeyIdBytes: MAX_KEY_ID_BYTES,
  maxPathBytes: MAX_PATH_BYTES,
  maxExpiresDigits: MAX_EXPIRES_DIGITS,
  methods: METHODS,
});

export default {
  authorize: authorize,
  loadMediaGrantKey: loadMediaGrantKey,
  loadMediaGrantKeyRing: loadMediaGrantKeyRing,
  loadMediaGrantKeyRingFromEnv: loadMediaGrantKeyRingFromEnv,
  protocol: protocol,
  responseHeaders: responseHeaders,
  signingInput: signingInput,
  validMediaPath: validMediaPath,
  verifyMediaGrant: verifyMediaGrant,
};
