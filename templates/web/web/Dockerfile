# Run `corepack pnpm build` first. The Helm chart's `web` component supplies
# the nginx config that sends dist/csp.txt as the Content-Security-Policy.
FROM nginxinc/nginx-unprivileged:1.29.7-alpine
COPY dist/ /usr/share/nginx/html/
