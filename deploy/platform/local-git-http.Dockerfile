FROM alpine:3.24.4@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf

RUN apk add --no-cache git git-daemon python3 \
  && addgroup -S -g 65532 git-http \
  && adduser -S -D -H -u 65532 -G git-http git-http
RUN git config --system --add safe.directory /srv/git/baukit.git
COPY local-git-http.py /usr/local/bin/local-git-http

USER 65532:65532
ENTRYPOINT ["python3", "/usr/local/bin/local-git-http"]
