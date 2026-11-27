# Tiny app for the baukit-app chart in the e2e cluster. It answers the chart's
# probes on the ops port and a fixed body on the http port.
ARG NGINX_IMAGE=nginx/nginx-unprivileged:1.31.10-alpine@sha256:86628f2c9eaa5e84e3b98d25e26563c4a4f9b0ead9cf5656da4731d86e0621de
FROM ${NGINX_IMAGE}
# faulty.conf builds the image the canary e2e rolls back.
ARG ECHO_CONF=default.conf
COPY ${ECHO_CONF} /etc/nginx/conf.d/default.conf
USER 101
