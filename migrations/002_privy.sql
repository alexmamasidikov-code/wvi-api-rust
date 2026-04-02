-- Add Privy DID to users table
ALTER TABLE users ADD COLUMN IF NOT EXISTS privy_did VARCHAR(255) UNIQUE;
ALTER TABLE users ADD COLUMN IF NOT EXISTS linked_accounts JSONB DEFAULT '[]';
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- Index for Privy DID lookups
CREATE INDEX IF NOT EXISTS idx_users_privy_did ON users(privy_did);
