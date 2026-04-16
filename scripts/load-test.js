import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  stages: [
    { duration: '30s', target: 100 },   // ramp up
    { duration: '1m', target: 500 },    // sustained load
    { duration: '30s', target: 1000 },  // peak
    { duration: '30s', target: 0 },     // ramp down
  ],
  thresholds: {
    http_req_duration: ['p(95)<500', 'p(99)<1000'],
    http_req_failed: ['rate<0.01'],
  },
};

const BASE_URL = __ENV.BASE_URL || 'http://localhost:8091';
const TOKEN = 'Bearer dev-token';

export default function () {
  // Health check
  const health = http.get(`${BASE_URL}/api/v1/health/server-status`);
  check(health, { 'health 200': (r) => r.status === 200 });

  // WVI current
  const wvi = http.get(`${BASE_URL}/api/v1/wvi/current`, {
    headers: { Authorization: TOKEN },
  });
  check(wvi, { 'wvi 200': (r) => r.status === 200 });

  // Biometrics sync
  const syncPayload = JSON.stringify({
    records: [
      { type: 'heart_rate', timestamp: new Date().toISOString(), data: { bpm: 72 } },
      { type: 'spo2', timestamp: new Date().toISOString(), data: { value: 98 } },
    ],
  });
  const sync = http.post(`${BASE_URL}/api/v1/biometrics/sync`, syncPayload, {
    headers: { Authorization: TOKEN, 'Content-Type': 'application/json' },
  });
  check(sync, { 'sync 200': (r) => r.status === 200 });

  // Emotions
  const emotions = http.get(`${BASE_URL}/api/v1/emotions/current`, {
    headers: { Authorization: TOKEN },
  });
  check(emotions, { 'emotions 200': (r) => r.status === 200 });

  sleep(0.1);
}
