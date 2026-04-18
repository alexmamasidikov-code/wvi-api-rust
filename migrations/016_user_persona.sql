-- Persona-based HOME layout (Sprint 3 of NPS uplift roadmap).
--
-- Each user picks one of four personas during onboarding (or later via
-- Settings); the choice reorders HOME sections so the most relevant
-- metrics for that user surface first. Stored on the users table as a
-- nullable text column rather than a separate table because:
--   * 1:1 with users
--   * Almost always read alongside the user row
--   * Simpler partial-update path
--
-- Allowed values: 'athlete' | 'professional' | 'parent' | 'curious'.
-- Enforced in the application layer (PUT /api/v1/users/me/persona) so we
-- can extend the set without a new migration.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS persona TEXT;
