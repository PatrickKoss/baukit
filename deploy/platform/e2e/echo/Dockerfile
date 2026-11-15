# Tiny app for the baukit-app chart in the e2e cluster. It answers the chart's
# probes on the ops port and a fixed body on the http port.
ARG NGINX_IMAGE=nginxinc/nginx-unprivileged:1.29.3-alpine@sha256:5aea7cc516b419e3526f47dd1531be31a56a046cfe44754d94f9383e13e2ee99
FROM ${NGINX_IMAGE}
COPY default.conf /etc/nginx/conf.d/default.conf
