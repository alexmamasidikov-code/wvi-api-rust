# APNs production setup

The Wellex Rust API uses Apple Push Notification Service to deliver:
- BP / ECG crisis alerts (Projects D & F)
- Sensitivity critical signals (Project B)
- Reminder fires (water / stand / breathe / bedtime / move / wvi_drop — Project E)
- Morning + evening AI briefs (Project C narrator crons)

Until the VPS env vars are populated, all those paths degrade to a one-line
warning in the logs (`APNs: env not set — pushes will be no-op`). No crashes.
The logic is in `src/push/apns.rs`.

## What you need

1. **APNs Auth Key (.p8 file)** — Apple Developer → Certificates, Identifiers & Profiles → Keys → "+" → enable APNs capability → download the `.p8`.
2. **Key ID** (10-char string) — shown next to the key on the Developer portal.
3. **Team ID** (10-char string) — Membership → Team ID.
4. **Bundle Identifier** — `com.wvi.health` for WVI Health.

## Install on VPS

```bash
ssh -i ~/.ssh/wvi_deploy alex@100.90.71.111
cd /home/alex/wvi-api-rust
# Append APNs vars to docker-compose .env (the container loads it at boot).
cat >> .env <<EOF
APNS_KEY_P8=$(cat ~/Downloads/AuthKey_ABCDEF1234.p8 | base64 | tr -d '\n')
APNS_KEY_ID=ABCDEF1234
APNS_TEAM_ID=8VBJ3K8A3S
APNS_BUNDLE_ID=com.wvi.health
EOF
# Restart the api container so it picks the new env.
docker compose up -d --no-deps api
docker logs --tail 20 wvi-api | grep -i apns
```

Expected log line on startup after env is set:
```
INFO wvi_api::push::apns: APNs configured for production key=ABCDEF… team=8VBJ3K8A3S bundle=com.wvi.health
```

## End-to-end verify

```bash
# Trigger a BP crisis to test the full push path.
curl -X POST -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \
  -d '{"records":[{"timestamp":"2026-04-18T20:00:00Z","systolic":185,"diastolic":125,"source":"manual"}]}' \
  https://6ssssdj5s38h.share.zrok.io/api/v1/biometrics/blood-pressure
```

Within ≤ 5s the phone should receive a push titled
"⚠️ Критическое давление 185/125 — обратись к врачу".

## Rotate keys

If the .p8 is revoked on the Developer portal, update the 3 env vars and
restart:
```bash
docker compose up -d --no-deps api
```

No other config / code changes needed.
