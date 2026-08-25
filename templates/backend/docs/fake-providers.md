# Fake providers

Read this before wiring a third-party HTTP integration. It is documentation
only; nothing in the generated code depends on it, and a backend that calls no
external provider can ignore it.

Integration UX is hard to exercise against a live provider: the sandbox needs
credentials, rate limits are real, and a failure path you want to see once is
not something you can ask a provider to produce on demand. A WireMock container
in a Compose override solves both halves. It serves scripted provider responses,
and one environment variable picks which failure the product's own connector
returns.

## Compose override

Keep the fake stack in a second Compose file so nothing about it can reach a
deployed environment. It is only active when you name it.

```yaml
# compose.fake-providers.yaml
services:
  fake-providers:
    image: wiremock/wiremock:3.13.2
    entrypoint:
      ["/docker-entrypoint.sh", "--global-response-templating", "--disable-gzip"]
    ports:
      - "127.0.0.1:${FAKE_PROVIDER_PORT:-{{ context.fake_provider_host_port }}}:8080"
    volumes:
      - ./fake-providers:/home/wiremock:ro
    healthcheck:
      test:
        - CMD
        - curl
        - --fail
        - --silent
        - --output
        - /dev/null
        - http://127.0.0.1:8080/__admin/mappings
      interval: 2s
      timeout: 2s
      retries: 20

  api:
    environment:
      {{ context.app_env }}__PROVIDERS__FAKE_SCENARIO: ${FAKE_SCENARIO:-healthy}
    depends_on:
      fake-providers:
        condition: service_healthy
```

Run it with `docker compose -f compose.yaml -f compose.fake-providers.yaml up -d`
and tear it down with the same file list plus `down`.

Two URL families, and mixing them up is the usual first failure:

- **Authorization URLs** point at `http://localhost:${FAKE_PROVIDER_PORT}`. The
  browser resolves them, so they must be reachable from the host.
- **Token and API base URLs** point at `http://fake-providers:8080`. The backend
  resolves them inside the Compose network and must not detour through the host.

Bind the published port to `127.0.0.1`. Nothing outside the machine has any
reason to reach a fixture server.

## FAKE_SCENARIO

`FAKE_SCENARIO` names one scripted outcome for the whole stack. Parse it once,
into an enum with a `healthy` default, and let every consumer read that enum
rather than re-matching the string.

| Scenario | Connector behavior |
|---|---|
| `healthy` | Every call succeeds. |
| `rate_limited` | First attempt is retryable with a retry-after; later attempts succeed. |
| `unavailable` | First attempt is retryable; later attempts succeed. |
| `timeout` | First attempt is retryable; later attempts succeed. |
| `revoked` | First attempt reports the credential as revoked; later attempts succeed. |
| `exhausted` | Every attempt stays retryable, so the attempt cap ends the job. |

The one-shot shape is deliberate. A transient scenario that failed forever would
only ever prove the retry path, never the recovery path. `exhausted` is the
exception because its whole purpose is to reach the terminal `failed` state with
an `attempts_exhausted` reason.

Anything a scenario returns is a fixture. Never reuse one of these tokens,
secrets, or signing values in a deployed environment.

## Stub mappings

WireMock loads `mappings/` from the mounted directory at startup, so a product
adds a provider by adding a JSON file. Keep them flat.

```text
fake-providers/
└── mappings/
    ├── oauth-authorize.json
    ├── <provider>-token.json
    └── <provider>-profile.json
```

One shared authorize stub covers every provider's browser redirect. With
`--global-response-templating` it echoes the caller's `redirect_uri` and `state`
back, so an OAuth connect flow completes with no provider login at all.

```json
{
  "name": "Fake OAuth authorization redirect",
  "priority": 1,
  "request": {
    "method": "GET",
    "urlPathPattern": "/(provider-a|provider-b)/oauth/authorize"
  },
  "response": {
    "status": 302,
    "headers": {
      "Location": "{{'{{{'}}request.query.redirect_uri{{'}}}'}}?code=fake-code&state={{'{{{'}}request.query.state{{'}}}'}}"
    },
    "transformers": ["response-template"]
  }
}
```

The rest are ordinary request-to-response pairs: a token endpoint returning a
fixed access and refresh token with a far-future expiry, a profile endpoint
returning a fixed identity. The production HTTP serialization still runs against
them, which is the point. Only the peer is fake.
