-- A production deployment must open each public capability deliberately.
-- This migration also closes an existing prelaunch singleton whose 0041
-- defaults were permissive. Platform admins reopen controls after the live
-- two-business, billing, delivery and restore gates pass.
ALTER TABLE platform_controls
  ALTER COLUMN public_signup_enabled SET DEFAULT FALSE,
  ALTER COLUMN business_creation_enabled SET DEFAULT FALSE,
  ALTER COLUMN plan_ordering_enabled SET DEFAULT FALSE,
  ALTER COLUMN domain_onboarding_enabled SET DEFAULT FALSE,
  ALTER COLUMN mailbox_provisioning_enabled SET DEFAULT FALSE,
  ALTER COLUMN outbound_sending_enabled SET DEFAULT FALSE;

UPDATE platform_controls
SET public_signup_enabled=FALSE,
    business_creation_enabled=FALSE,
    plan_ordering_enabled=FALSE,
    domain_onboarding_enabled=FALSE,
    mailbox_provisioning_enabled=FALSE,
    outbound_sending_enabled=FALSE,
    maintenance_message='CS Mail is completing production acceptance.',
    updated_at=now()
WHERE singleton=TRUE;
