-- Accounts can be disabled from the user management page instead of deleted, so their
-- containers keep an owner. Existing users stay active.
ALTER TABLE users ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1;
