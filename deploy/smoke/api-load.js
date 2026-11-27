// Drives a generated API past its concurrency limit. Passes when every shed response is a 503 with
// Retry-After and the ops listener answers /healthz throughout and after the load. The admitted p99
// is checked by admitted_latency.py from the CSV samples, after correcting for wall clock steps the
// `clock` scenario records.
import { sleep } from "k6";
import http from "k6/http";
import { Counter, Rate, Trend } from "k6/metrics";

const env = (name, fallback) => __ENV[name] || fallback;
const number = (name, fallback) => Number(env(name, String(fallback)));

const API_URL = env("API_URL", "http://127.0.0.1:8080");
const OPS_URL = env("OPS_URL", "http://127.0.0.1:9090");
const SMOKE_PATH = env("SMOKE_PATH", "/items");
const SMOKE_TOKEN = env("SMOKE_TOKEN", "");
const START_RPS = 50;
const PEAK_RPS = number("PEAK_RPS", 2000);
const RAMP_SECONDS = number("RAMP_SECONDS", 10);
const HOLD_SECONDS = number("HOLD_SECONDS", 20);
const MAX_VUS = number("MAX_VUS", 400);
const LIVENESS_AFTER_SECONDS = 5;
const REQUEST_TIMEOUT = "10s";
const CLOCK_TICK_SECONDS = 0.1;
const MS_PER_SECOND = 1000;

const ADMITTED = 200;
const SHED = 503;

const admittedDuration = new Trend("admitted_duration", true);
const shedResponses = new Counter("shed_responses");
const shedWithoutRetryAfter = new Rate("shed_without_retry_after");
const unexpectedStatus = new Rate("unexpected_status");
const processUp = new Rate("process_up");
const clockLag = new Trend("clock_lag", true);

const params = {
  timeout: REQUEST_TIMEOUT,
  headers: SMOKE_TOKEN ? { Authorization: `Bearer ${SMOKE_TOKEN}` } : {},
};

export const options = {
  discardResponseBodies: true,
  scenarios: {
    overload: {
      executor: "ramping-arrival-rate",
      exec: "overload",
      startRate: START_RPS,
      timeUnit: "1s",
      preAllocatedVUs: START_RPS,
      maxVUs: MAX_VUS,
      stages: [
        { target: PEAK_RPS, duration: `${RAMP_SECONDS}s` },
        { target: PEAK_RPS, duration: `${HOLD_SECONDS}s` },
      ],
    },
    clock: {
      executor: "constant-vus",
      exec: "clock",
      vus: 1,
      duration: `${RAMP_SECONDS + HOLD_SECONDS + LIVENESS_AFTER_SECONDS}s`,
    },
    liveness: {
      executor: "constant-arrival-rate",
      exec: "liveness",
      rate: 1,
      timeUnit: "1s",
      duration: `${RAMP_SECONDS + HOLD_SECONDS + LIVENESS_AFTER_SECONDS}s`,
      preAllocatedVUs: 2,
    },
  },
  thresholds: {
    shed_responses: ["count>0"],
    shed_without_retry_after: ["rate==0"],
    unexpected_status: ["rate==0"],
    process_up: ["rate==1"],
  },
};

export function overload() {
  const response = http.get(`${API_URL}${SMOKE_PATH}`, params);
  unexpectedStatus.add(response.status !== ADMITTED && response.status !== SHED);
  if (response.status === ADMITTED) {
    admittedDuration.add(response.timings.duration);
    return;
  }
  if (response.status === SHED) {
    shedResponses.add(1);
    shedWithoutRetryAfter.add(!/^[0-9]+$/.test(response.headers["Retry-After"] || ""));
  }
}

export function liveness() {
  const response = http.get(`${OPS_URL}/healthz`, { timeout: REQUEST_TIMEOUT });
  processUp.add(response.status === ADMITTED);
}

export function clock() {
  const started = Date.now();
  sleep(CLOCK_TICK_SECONDS);
  clockLag.add(Date.now() - started - CLOCK_TICK_SECONDS * MS_PER_SECOND);
}
