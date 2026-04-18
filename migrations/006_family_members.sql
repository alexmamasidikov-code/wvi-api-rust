-- Family/circle relations. owner_id and member_id reference privy_did (TEXT),
-- not users.id, because invites may precede account creation.

CREATE TABLE IF NOT EXISTS family_members (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id TEXT NOT NULL,         -- privy_did of the user who owns this relation
    member_id TEXT,                  -- nullable until accepted
    invite_email TEXT,
    display_name TEXT NOT NULL,
    relation TEXT NOT NULL,          -- 'self' | 'spouse' | 'child' | 'parent' | 'sibling' | 'other'
    status TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'accepted' | 'declined'
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_family_owner ON family_members(owner_id);
CREATE INDEX IF NOT EXISTS idx_family_member ON family_members(member_id);
