# Minimal API

A small, standalone notes service that shows how a product composes all six Baukit crates. State is process-local and in memory; there is deliberately no authentication, database, or container setup.

## Run it

From this directory:

```sh
cargo run
```

The public API listens on `0.0.0.0:8080`; private operations endpoints listen on `0.0.0.0:9090`. Configuration uses the `MINIMAL_API__` prefix and double underscores for nesting:

```sh
MINIMAL_API__HTTP__PORT=8081 \
MINIMAL_API__OPS__PORT=9091 \
MINIMAL_API__MAX_NOTES=25 \
cargo run
```

`MINIMAL_API_ENVIRONMENT=local|staging|production` (or `--environment`) selects bootstrap behavior and log format. Staging and production also require `OTEL_EXPORTER_OTLP_ENDPOINT`. Local runs may use an optional `config/local.toml` and `.env`.

Try the API:

```sh
curl -i -X POST http://localhost:8080/notes \
  -H 'content-type: application/json' \
  -H 'x-request-id: demo-1' \
  -d '{"title":"Read Baukit docs","body":"Start with each crate root."}'
curl -i http://localhost:8080/notes/1
curl -i http://localhost:8080/notes
curl -i http://localhost:8080/fail
curl -i http://localhost:9090/healthz
curl -i http://localhost:9090/readyz
curl -i http://localhost:9090/metrics
```

Generate the deterministic schema with `cargo run -- --openapi`. Set `OPENAPI_OUT` to choose another output path. `cargo test` checks the committed `openapi.json` for drift.

## What is wired where

- `baukit-config` layers defaults, optional local files, and `MINIMAL_API__...` environment values into `BaukitConfig<ProductConfig>`; `max_notes` is the product-owned field and the unused database section stays absent.
- `baukit-telemetry` installs local/deployed logging, W3C propagation, tracing, the Prometheus recorder, and `build_info`; it is explicitly flushed during drain.
- `baukit-http` finalizes every public route with normalized extractor/404/405 errors, request IDs, trace spans, limits, CORS, panic/timeout handling, and the three standard HTTP metrics.
- `baukit-ops` owns the separate `/healthz`, `/readyz`, `/metrics`, and `/buildinfo` router. Readiness includes the in-memory state and the drain traffic gate.
- `baukit-runtime` supplies build identity, two-listener serving, signal-driven shutdown, automatic readiness-gate closure, one shared drain deadline, and supervision of the periodic note-count janitor.
- `baukit-openapi` applies shared metadata and error schemas, writes stable JSON, and checks the committed document for drift.
