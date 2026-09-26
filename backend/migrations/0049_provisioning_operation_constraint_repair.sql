-- Upgrade 49: repair the durable provisioning operation contract.
--
-- Some long-lived production databases were created while the provisioning
-- operation CHECK constraint only knew the older operation set. The current
-- application legitimately queues `set_access` when subscriptions are
-- suspended/reactivated and during provider reconciliation. Reassert the
-- complete operation contract in a new forward-only migration instead of
-- relying on the historical migration having the same on-disk definition.

ALTER TABLE provisioning_jobs
  DROP CONSTRAINT IF EXISTS provisioning_jobs_operation_check;

ALTER TABLE provisioning_jobs
  ADD CONSTRAINT provisioning_jobs_operation_check
  CHECK (operation IN (
    'ensure_mailbox',
    'set_quota',
    'set_credentials',
    'set_access',
    'delete_mailbox'
  ));

COMMENT ON CONSTRAINT provisioning_jobs_operation_check ON provisioning_jobs IS
  'Provider job operations supported by the CS Mail provisioning worker; repaired by migration 0049 for long-lived production databases.';
