-- Bridge key to the Stalwart mail store (WS2): the per-user Stalwart account
-- id, populated when a mailbox is provisioned on signup (or lazily on first
-- mail request). NULL/empty = user has no mailbox on the primary domain.
ALTER TABLE users ADD COLUMN IF NOT EXISTS mail_account_id TEXT;