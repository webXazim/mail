# CrescentSphere DNS rebuild

`crescentsphere.com.import.txt` is a Cloudflare BIND import file derived from
the zone export of 2026-09-23. It contains 22 records: the active websites,
Stalwart transport, and the two Mailer sending domains already in use. Cloudflare
owns the zone SOA and nameservers, so they are omitted.

## Cutover

1. Keep the original Cloudflare export as the rollback copy. Confirm Stalwart
   still owns `crescentsphere.com`, accepts mail for the required root addresses,
   and listens on public TCP 25. Confirm the VPS PTR remains
   `smtp.crescentsphere.com`.
2. In Cloudflare Email Routing, disable routing for `crescentsphere.com`. This
   removes its managed root MX, SPF, and `cf2024-1` DKIM records. Root inbound
   email will then go to Stalwart after the new MX is published.
3. For a full rebuild, remove the remaining editable records and immediately
   import `crescentsphere.com.import.txt` via DNS > Records > Import and Export.
   Importing a zone file does not delete old records. Website and mail names
   will be unavailable between deletion and re-import; Cloudflare retains its
   zone SOA and nameservers automatically.
4. Check the import result for rejected records; the tunnel CNAMEs and the
   `cf-proxied` tags must keep their original proxy states.
5. Re-export the zone and compare it to this file. Verify the root MX, SPF,
   DMARC, and DNS-only `smtp` host, then test inbound and outbound mail.

The file removes the unused mail-client discovery records under
`mailer.crescentsphere.com`, stale ACME TXT values, Resend DKIM, unused
MTA-STS/TLS reporting records for that host, and duplicate Mailer DMARC/DKIM
records. The Cloudflare OAuth publisher TXT remains because Mailer's DNS
automation may still use that integration.

## Root-domain sending through Mailer

Mailer supports this through an explicitly configured shared-domain path.
Keep CS Mail's existing Stalwart root domain and mailboxes. Set
`STALWART_SHARED_DOMAIN=crescentsphere.com` and
`STALWART_SHARED_WORKSPACE_ID` to the chosen Mailer workspace UUID in the
Mailer production environment, deploy Mailer, then add the root domain in that
workspace. Publish the exact new `_mailer-verification.crescentsphere.com`,
`<selector>._domainkey.crescentsphere.com`, and
`bounce.crescentsphere.com` MX/SPF values shown in Mailer. They cannot be
invented in advance. The initial root SPF in this file authorizes the existing
Stalwart host for CS Mail's direct root-domain messages; retain one SPF record
at the root when adding Mailer.
