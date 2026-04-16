# Deployment Guide

## Prerequisites
- Docker + Docker Compose
- PostgreSQL 16
- Redis 7 (optional, for caching)

## Quick Start
1. Clone: `git clone <repo>`
2. Configure: `cp .env.production .env` and edit secrets
3. Run: `docker compose up -d`
4. Check: `curl http://localhost:8091/api/v1/health/server-status`

## Environment Variables
| Variable | Description | Default |
|----------|-------------|---------|
| DATABASE_URL | PostgreSQL connection | required |
| PORT | API port | 8091 |
| CLAUDE_API_KEY | OpenRouter key | optional |
| MAX_DB_CONNECTIONS | Pool size | 20 |
| RUST_LOG | Log level | info |

## iOS App
1. Install XcodeGen: `brew install xcodegen`
2. Generate: `xcodegen generate`
3. Build: `xcodebuild -scheme WVIHealth ...`
4. Deploy: `xcrun devicectl device install app ...`
