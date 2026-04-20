#!/bin/bash
# WVI AI Analytics Agent
# Runs via Claude Code CLI every 15 minutes
# Analyzes biometric data and updates insights in the database

DB='postgres://wvi:wvi_secure_2026@127.0.0.1:5433/wvi'
API='http://localhost:8091'
TOKEN='Bearer dev-token'

echo "=== WVI AI Analytics Run: $(date) ==="

# Fetch current data
WVI=$(curl -s -H "Authorization: $TOKEN" $API/api/v1/wvi/current)
EMOTION=$(curl -s -H "Authorization: $TOKEN" $API/api/v1/emotions/current)
RECOVERY=$(curl -s -H "Authorization: $TOKEN" $API/api/v1/biometrics/recovery)
REALTIME=$(curl -s -H "Authorization: $TOKEN" $API/api/v1/biometrics/realtime)
COMPUTED=$(curl -s -H "Authorization: $TOKEN" $API/api/v1/biometrics/computed)

echo "WVI: $(echo $WVI | python3 -c 'import sys,json; d=json.load(sys.stdin)["data"]; print(f"{d["wviScore"]} ({d["level"]})")')"
echo "Emotion: $(echo $EMOTION | python3 -c 'import sys,json; d=json.load(sys.stdin)["data"]; print(d["primary"] if d else "none")')"
echo "Recovery: $(echo $RECOVERY | python3 -c 'import sys,json; d=json.load(sys.stdin)["data"]; print(f"{d["recoveryPercent"]}%")')"

echo "Analytics complete."
