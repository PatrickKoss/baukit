FROM alpine:3.22

RUN apk add --no-cache git git-daemon python3
RUN git config --system --add safe.directory /srv/git/baukit.git
COPY local-git-http.py /usr/local/bin/local-git-http

ENTRYPOINT ["python3", "/usr/local/bin/local-git-http"]
