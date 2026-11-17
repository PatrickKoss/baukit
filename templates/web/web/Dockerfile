# Run `corepack pnpm build` first. The Helm chart's `web` component supplies
# the nginx config that sends dist/csp.txt as the Content-Security-Policy, next
# to COOP and CORP `same-origin`.
FROM nginx/nginx-unprivileged:1.31.8-alpine@sha256:f90ef283a8a580912aa56315c3ed8f54cf50c2e89ed7e6fbf4c53368b55d712f
COPY dist/ /usr/share/nginx/html/
USER 101
