"""Logs one line per alert of each Alertmanager post and serves the latest posts on GET /alerts."""

import json
import threading
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MAX_BODY_BYTES = 1 << 20
PORT = 8080
KEPT_NOTIFICATIONS = 500

lock = threading.Lock()
notifications = deque(maxlen=KEPT_NOTIFICATIONS)


def log_alerts(notification):
    for alert in notification.get("alerts", []):
        labels = alert.get("labels", {})
        line = {
            "receiver": notification.get("receiver"),
            "status": alert.get("status"),
            "alertname": labels.get("alertname"),
            "namespace": labels.get("namespace"),
            "severity": labels.get("severity"),
        }
        print(json.dumps(line), flush=True)


class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body=b""):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/healthz":
            self.reply(200, b"{}")
        elif self.path == "/alerts":
            with lock:
                body = json.dumps(list(notifications)).encode()
            self.reply(200, body)
        else:
            self.reply(404)

    def do_POST(self):
        if self.path != "/alerts":
            self.reply(404)
            return
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            self.reply(411)
            return
        if not 0 <= length <= MAX_BODY_BYTES:
            self.reply(413)
            return
        try:
            notification = json.loads(self.rfile.read(length))
        except json.JSONDecodeError:
            self.reply(400)
            return
        with lock:
            notifications.append(notification)
        log_alerts(notification)
        self.reply(200, b"{}")


ThreadingHTTPServer(("", PORT), Handler).serve_forever()
