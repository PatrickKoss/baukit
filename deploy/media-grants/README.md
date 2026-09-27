# Media grant verifier for nginx

`njs/media-grant.js` checks signed media grants at an nginx edge in front of object storage or a
volume of immutable media. The backend signs grants with `baukit-core`'s `media-grants` feature.
Both sides pass `fixtures/media-grants/vectors-v1.json`.

A grant is the query `expires=<unix seconds>&keyId=<id>&mode=playback&signature=<base64url>` on a
media path. The signature is unpadded base64url of HMAC-SHA256 over
`"{path}\n{expires}\nplayback\n{keyId}"`, keyed with the base64url-decoded secret.

## What the verifier checks

In this order, each failure returning 403:

1. The method is `GET` or `HEAD`.
2. The raw path from `$request_uri` equals nginx's normalized `$uri`, so percent-encoding and dot
   segments never reach the signature check.
3. The path is at most 512 bytes of `/`-separated segments made of `[A-Za-z0-9._-]`, none empty
   and none starting with `.`.
4. The query holds exactly `expires`, `keyId`, `mode=playback`, and `signature`, in that order.
   `expires` has one to ten digits with no leading zero, and the signature is 43 canonical
   base64url characters.
5. The grant has not expired and expires at most 3660 seconds ahead: the 3600-second signing
   limit plus 60 seconds for a verifier clock that lags the signer's.
6. `keyId` names the current or the previous key.
7. The HMAC matches. The comparison runs over all 32 bytes without an early exit.

On success the handler sets `$media_grant_cache_control` to
`private, max-age=<seconds left>, must-revalidate`. The header filter copies it into
`Cache-Control` for responses below 400 and removes it from errors.

The verifier does not know which paths a product serves. Put that allowlist in `location` blocks.

## Configuration

The keys come from the process environment, so no nginx variable holds them and no `log_format`
can print them:

| Variable | Required | Value |
|---|---|---|
| `MEDIA_GRANT_KEY_ID` | yes | Current key ID, `[A-Za-z0-9][A-Za-z0-9_-]*`, at most 64 bytes |
| `MEDIA_GRANT_SIGNING_KEY` | yes | Current secret, unpadded base64url of at least 32 bytes |
| `MEDIA_GRANT_PREVIOUS_KEY_ID` | during rotation | Previous key ID |
| `MEDIA_GRANT_PREVIOUS_SIGNING_KEY` | during rotation | Previous secret |

Set both previous variables or neither. A broken key configuration refuses every request and
writes `media grant keys are invalid: <code>` to the error log, never the value.

```nginx
load_module /usr/lib/nginx/modules/ngx_http_js_module.so;
env MEDIA_GRANT_KEY_ID;
env MEDIA_GRANT_SIGNING_KEY;
env MEDIA_GRANT_PREVIOUS_KEY_ID;
env MEDIA_GRANT_PREVIOUS_SIGNING_KEY;

events {}

http {
    js_engine qjs;
    js_import media_grant from /etc/nginx/njs/media-grant.js;
    js_var $media_grant_cache_control 'no-store';

    # Log $uri, not $request or $request_uri: the query is a bearer credential.
    log_format media '$request_method $uri $status $body_bytes_sent $request_time';
    access_log /dev/stdout media;

    server {
        listen 8080;
        root /srv/media;

        location ~ ^/media/[0-9a-f-]+/v[0-9]+/[a-z0-9-]+\.(mp4|jpg)$ {
            js_access media_grant.authorize;
            js_header_filter media_grant.responseHeaders;
            try_files $uri =404;
        }
        location / { return 404; }
    }
}
```

Keep access logs, ingress logs, and tracing from recording the query. An ingress in front of
nginx must forward the request target unchanged; rewriting or re-encoding the path makes the
raw-path check fail.

To rotate keys, move the current key into the previous variables, set a new current key, and
roll the backend and the edge. Once the longest grant lifetime has passed, clear the previous
variables.

## Runtimes

The module is tested with njs 1.0.1 in nginx 1.31.6 under both engines (`js_engine njs` and
`js_engine qjs`) and in Node 24. The native engine has no destructuring, `for...of`, classes, or
named exports, so the module uses none of them and exports one default object. It needs `Buffer`
with `base64url`, `crypto.subtle` HMAC, and `process.env`.

## Tests

`make media-grants-test` runs `njs/media-grant.test.mjs` under Node. Node already has `Buffer`,
`crypto.subtle`, and `process.env`, so the only shims are a fake nginx request object and mocked
`Date` for the handler tests, plus `njs/package.json`, which marks the files as ES modules so
Node does not warn. nginx ignores it.

`make media-grants-njs-test` runs `njs/run-njs-vectors.js` in the pinned `nginx:1.31-alpine`
image under both engines. `njs/vector-checks.js` holds the checks both runners share.
