-- Add admin flag to accounts table for debug/development features
ALTER TABLE accounts ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT FALSE;

-- Create index for quick admin lookups
CREATE INDEX idx_accounts_is_admin ON accounts(is_admin) WHERE is_admin = TRUE;

-- To promote an account to admin, run:
-- UPDATE accounts SET is_admin = TRUE WHERE email = 'your@email.com';
--
-- To list all admin accounts:
-- SELECT id, email FROM accounts WHERE is_admin = TRUE;
