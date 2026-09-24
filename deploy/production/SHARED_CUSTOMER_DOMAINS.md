# One customer domain for mailboxes and Mailer sending

CS Mail and CrescentSphere Mailer share one Stalwart Domain object for a DNS
name. Each product requires its own public TXT ownership proof. A matching name
inside Stalwart is not sufficient authorization.

For an existing Mailer domain such as `webxazim.com`, deploy the compatible
Mailer and CS Mail releases, including CS Mail migration 0044. In CS Mail,
verify the business domain's `_cs-mail-verify` TXT challenge, then provision
it. CS Mail stores a `shared_mailer_domain` binding and leaves Mailer's Stalwart
description, bounce alias, and DKIM signature intact. Publish the Stalwart
zone's apex MX, SPF, DKIM, and DMARC records without replacing conflicting
records blindly. Check DNS and create the first mailbox only when the domain is
active. Run inbound, outbound, IMAP, and Mailer API send tests before moving
production traffic.

For a domain first created in CS Mail, add it in Mailer, publish the displayed
`_mailer-verification` TXT challenge, and verify it. Mailer then attaches its
bounce alias and separate DKIM signature. Publish the remaining Mailer records
and verify again. Existing CS Mail apex records must remain intact.

Removing the domain from Mailer does not disable the Stalwart domain. Releasing
a customer domain in CS Mail removes only the CS Mail claim after its mailboxes
and addresses have been removed. The Stalwart domain remains in place even if
CS Mail created it. An operator must inspect both products before cleaning up
such a provider entry. Never delete a provider domain merely to clear an
ownership error.

## Recovery after DNS records or a Mailer claim are removed

If a Mailer sending domain is disabled and then added again, its new Mailer
claim has a new `_mailer-verification.<domain>` TXT value. Publish the value
currently shown in Mailer; the old value cannot verify the new claim. Mailer
may reuse the existing Stalwart Domain object after this fresh public proof.
The disabled historical Mailer row retains its provider ID for audit and does
not block the new active binding.

Once Mailer shows its DKIM and `bounce.<domain>` MX/SPF records, publish those
records and verify again. They are separate from CS Mail's mailbox records.
Use CS Mail Business admin to retry provisioning, then publish its displayed
apex MX/SPF, DKIM, and DMARC records and run Check DNS. Mailer does not
automatically publish its optional DMARC suggestion, so CS Mail can manage
the single DMARC policy for the shared domain. Keep one SPF policy per DNS
name; do not replace the apex policy with the bounce policy.

Deleting DNS records does not clear either product's domain claim. CS Mail
degrades an active mailbox domain when required records disappear; Mailer also
rechecks verified sending domains and returns them to pending when required
records disappear. Restore the records and recheck each product before sending
or creating mailboxes.
