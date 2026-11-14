FROM alpine:3.24.2

RUN apk add --no-cache git git-daemon python3 \
  && addgroup -S -g 65532 git-http \
  && adduser -S -D -H -u 65532 -G git-http git-http
RUN git config --system --add safe.directory /srv/git/baukit.git
COPY local-git-http.py /usr/local/bin/local-git-http

USER 65532:65532
ENTRYPOINT ["python3", "/usr/local/bin/local-git-http"]
