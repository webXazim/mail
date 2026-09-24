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
