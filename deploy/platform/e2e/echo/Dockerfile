# Tiny app for the baukit-app chart in the e2e cluster. It answers the chart's
# probes on the ops port and a fixed body on the http port.
ARG NGINX_IMAGE=nginx/nginx-unprivileged:1.31.8-alpine@sha256:f90ef283a8a580912aa56315c3ed8f54cf50c2e89ed7e6fbf4c53368b55d712f
FROM ${NGINX_IMAGE}
COPY default.conf /etc/nginx/conf.d/default.conf
USER 101
