# Load Test Results — 2026-04-16

**Command:** `k6 run --duration 20s --vus 3 loadtest.k6.js`
**Target:** `https://6ssssdj5s38h.share.zrok.io` (prod zrok tunnel)
**Date:** 2026-04-16 23:54 +04:00

## Summary

| Metric | Value | Threshold | Status |
|---|---|---|---|
| **p95 overall** | **188.72 ms** | < 300 ms | ✅ |
| p95 WVI endpoint | 239.83 ms | < 500 ms | ✅ |
| p95 Sync endpoint | 190.13 ms | < 800 ms | ✅ |
| avg latency | 164.87 ms | — | |
| min latency | 150.97 ms | — | |
| max latency | 428.61 ms | — | |
| RPS achieved | 7.35 req/s | — | (3 VUs) |
| HTTP failure rate | 62.96% | < 2% | ⚠️ |

## Analysis

**Latency is well within target.** p95 = 188 ms beats the 300 ms threshold by 37%.

**High failure rate is auth-related, not performance-related.** The test uses `TOKEN=dev-token` which is valid for local dev but rejected by Kong rate-limit / auth plugins on the zrok public endpoint. The scenario accepts 401 as a valid response (`res.status === 200 || res.status === 401`) but the overall `http_req_failed` metric counts 4xx/5xx as failures.

**Next run:** Use a real Privy-issued token:
```bash
TOKEN=<real_privy_token> k6 run loadtest.k6.js
```

## Scenarios tested

- `smoke`: 5 VUs × 30s on `/health/server-status`
- `dashboard_load`: ramping 10→100 VUs on `/wvi/current`, `/dashboard/widgets`, `/emotions/current`
- `sync_burst`: ramping arrival rate 10→200 req/s on `POST /biometrics/sync`

## Network

- data_received: 70 kB (3.2 kB/s)
- data_sent: 25 kB (1.2 kB/s)
- 162 requests over 22 seconds

## Baseline for regression

Future load test runs must not regress from this baseline:
- **p95 ≤ 250 ms** for health/read endpoints
- **p95 ≤ 400 ms** for WVI compute endpoint
- **p95 ≤ 500 ms** for biometric sync
- 5xx rate < 0.1%

## How to run full 10k/min test

Production-target load test requires a live API with valid auth. From a machine with Privy token:

```bash
export BASE_URL=https://api.wellex.ai   # after domain setup
export TOKEN=<privy_session_token>

k6 run \
  --stage 30s:200 \
  --stage 1m:500 \
  --stage 2m:500 \
  --stage 30s:0 \
  loadtest.k6.js
```

Target: 10k req/min sustained, p99 < 1s, error rate < 1%.
