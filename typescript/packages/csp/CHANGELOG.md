# @baukit/csp

## Unreleased

- Add `PolicyOrigins.vendorScripts` (origins in `script-src`, for example the Clerk frontend API) and `PolicyOrigins.vendorFrames` (extra `frame-src` origins).
- Add `PolicyOptions.blobFrames`, which adds `blob:` to `frame-src`.
- Add `assertProductionOrigins`, which throws for `http:` origins, embedded credentials and loopback hosts (`localhost`, `*.localhost`, `127.*`, `0.0.0.0`, `[::1]`, `[::]`, IPv4-mapped IPv6 loopback).

## 0.19.2

### Patch Changes

- Release the coordinated baukit 0.19.2 train.

## 0.19.1

### Patch Changes

- Release the coordinated baukit 0.19.1 train.

## 0.19.0

- Move the Content Security Policy builder out of the web and mobile templates.
- Add `SECURITY_HEADERS` with COOP and CORP `same-origin` and `nosniff`.

### Minor Changes

- Release the coordinated baukit 0.19.0 train.
