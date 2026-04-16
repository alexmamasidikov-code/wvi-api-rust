// k6 load test for WVI API
// Run: k6 run loadtest.k6.js
// Env: BASE_URL=https://6ssssdj5s38h.share.zrok.io (default), TOKEN=dev-token

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend, Rate } from 'k6/metrics';

const BASE = __ENV.BASE_URL || 'https://6ssssdj5s38h.share.zrok.io';
const TOKEN = __ENV.TOKEN || 'dev-token';

const headers = {
    'Authorization': `Bearer ${TOKEN}`,
    'Content-Type': 'application/json',
};

// Custom metrics per endpoint
const healthLatency = new Trend('latency_health');
const wviLatency = new Trend('latency_wvi');
const syncLatency = new Trend('latency_sync');
const errorRate = new Rate('errors');

export const options = {
    scenarios: {
        // Scenario 1: smoke — 5 VUs for 30s baseline
        smoke: {
            executor: 'constant-vus',
            vus: 5,
            duration: '30s',
            tags: { scenario: 'smoke' },
        },
        // Scenario 2: dashboard read-heavy — ramps to 100 VUs
        dashboard_load: {
            executor: 'ramping-vus',
            startTime: '35s',
            startVUs: 10,
            stages: [
                { duration: '30s', target: 50 },
                { duration: '30s', target: 100 },
                { duration: '30s', target: 100 },
                { duration: '15s', target: 0 },
            ],
            tags: { scenario: 'dashboard' },
        },
        // Scenario 3: biometric sync burst — 200 VUs posting
        sync_burst: {
            executor: 'ramping-arrival-rate',
            startTime: '3m',
            startRate: 10,
            timeUnit: '1s',
            preAllocatedVUs: 50,
            maxVUs: 300,
            stages: [
                { duration: '30s', target: 100 }, // 100 req/s
                { duration: '30s', target: 200 }, // 200 req/s
                { duration: '30s', target: 0 },
            ],
            tags: { scenario: 'sync_burst' },
        },
    },
    thresholds: {
        'http_req_duration{scenario:smoke}': ['p(95)<300'],
        'http_req_duration{scenario:dashboard}': ['p(95)<500', 'p(99)<1000'],
        'http_req_duration{scenario:sync_burst}': ['p(95)<800'],
        'http_req_failed': ['rate<0.02'],  // under 2% error rate overall
        errors: ['rate<0.05'],
    },
};

function randomBiometric() {
    const now = new Date().toISOString();
    return {
        records: [
            {
                type: 'heart_rate',
                timestamp: now,
                data: { bpm: Math.floor(60 + Math.random() * 50) },
            },
            {
                type: 'spo2',
                timestamp: now,
                data: { value: Math.floor(95 + Math.random() * 5) },
            },
            {
                type: 'hrv',
                timestamp: now,
                data: { rmssd: 30 + Math.random() * 60, heartRate: 70 },
            },
        ],
    };
}

export default function () {
    const scenario = __ENV.K6_SCENARIO || exec_scenario();
    if (scenario === 'smoke') {
        smokeIteration();
    } else if (scenario === 'dashboard') {
        dashboardIteration();
    } else if (scenario === 'sync_burst') {
        syncIteration();
    } else {
        // Fallback — mixed
        Math.random() < 0.7 ? dashboardIteration() : syncIteration();
    }
}

function exec_scenario() {
    // k6's exec.scenario.name — use env or fall back
    // eslint-disable-next-line no-undef
    return (typeof __VU !== 'undefined') ? (__ITER % 2 === 0 ? 'dashboard' : 'sync_burst') : 'smoke';
}

function smokeIteration() {
    const r = http.get(`${BASE}/api/v1/health/server-status`);
    healthLatency.add(r.timings.duration);
    check(r, { 'health 200': (res) => res.status === 200 });
    errorRate.add(r.status !== 200);
    sleep(1);
}

function dashboardIteration() {
    const r1 = http.get(`${BASE}/api/v1/wvi/current`, { headers });
    wviLatency.add(r1.timings.duration);
    const r2 = http.get(`${BASE}/api/v1/dashboard/widgets`, { headers });
    const r3 = http.get(`${BASE}/api/v1/emotions/current`, { headers });

    check(r1, { 'wvi ok': (res) => res.status === 200 || res.status === 401 });
    errorRate.add(r1.status >= 500);
    errorRate.add(r2.status >= 500);
    errorRate.add(r3.status >= 500);
    sleep(Math.random() * 2);
}

function syncIteration() {
    const payload = JSON.stringify(randomBiometric());
    const r = http.post(`${BASE}/api/v1/biometrics/sync`, payload, { headers });
    syncLatency.add(r.timings.duration);
    check(r, { 'sync ok or auth': (res) => res.status === 200 || res.status === 401 });
    errorRate.add(r.status >= 500);
    sleep(0.1);
}
