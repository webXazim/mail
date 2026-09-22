-- CS Mail brand refresh.
-- Existing migration files remain immutable; normalize only known default plan
-- names so upgrades from Harbor / CS Mailer and fresh installs converge on the
-- current CS Mail product name without touching customer-customized names.

UPDATE plans
SET name = 'CS Mail Solo', updated_at = now()
WHERE code = 'solo' AND name IN ('Harbor Solo', 'CS Mailer Solo');

UPDATE plans
SET name = 'CS Mail Team', updated_at = now()
WHERE code = 'team' AND name IN ('Harbor Team', 'CS Mailer Team');

UPDATE plans
SET name = 'CS Mail Business', updated_at = now()
WHERE code = 'business' AND name IN ('Harbor Business', 'CS Mailer Business');
