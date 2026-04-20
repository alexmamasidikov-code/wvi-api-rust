#!/bin/bash
cd /home/alex/wvi-api-rust
claude --model claude-opus-4-7 -p "Fetch health data from http://localhost:8091/api/v1/wvi/current and /api/v1/biometrics/realtime and /api/v1/emotions/current and /api/v1/biometrics/recovery using curl with header Authorization: Bearer dev-token. Analyze ALL metrics. Give 3-sentence insight with specific numbers. Then save to DB: psql postgres://wvi:wvi_secure_2026@127.0.0.1:5433/wvi -c INSERT INTO alerts" --allowedTools "Bash(curl*),Bash(psql*)" >> /var/log/wvi-analytics.log 2>&1
