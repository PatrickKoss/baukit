# Run `corepack pnpm build` first. The Helm chart's `web` component supplies
# the nginx config that sends dist/csp.txt as the Content-Security-Policy, next
# to COOP and CORP `same-origin`.
FROM nginx/nginx-unprivileged:1.31.10-alpine@sha256:86628f2c9eaa5e84e3b98d25e26563c4a4f9b0ead9cf5656da4731d86e0621de
COPY dist/ /usr/share/nginx/html/
USER 101
