-- 017: Apple App Reviewer support
-- Adds an is_reviewer flag on users so the iOS app can detect the
-- official reviewer account (apple-review@wellex.io) and:
--   1. skip the bracelet-pairing onboarding step
--   2. surface seeded biometric data instead of waiting for hardware
--
-- The flag is server-controlled — never set client-side. Reviewer login
-- is a normal Privy auth; we just toggle this row column for that one
-- account before each App Store submission, and clear it after approval.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS is_reviewer BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_users_is_reviewer
    ON users(is_reviewer)
    WHERE is_reviewer = TRUE;
