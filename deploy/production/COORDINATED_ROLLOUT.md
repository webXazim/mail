# Coordinated shared-VPS rollout

CS Mail (business mailboxes), Mailer (developer sending), Messenger, and
Stalwart are separate applications. Stalwart is the shared mail server and
outbound IP. The independent platform edge owns public web ports 80 and 443;
Mailer continues to use its Cloudflare Tunnel. Keep each project's database,
credentials, Compose lifecycle, and backups separate.

## Source and deployment order

1. Push the reviewed Mail `main`, Mailer `main`, and Messenger `axum` branches.
   The Messenger release source deployed on the VPS must contain the production
   Compose change that removes Nginx's public `80:80` and `443:443` ports.
   Pull all three before starting the cutover. Confirm the exact commit SHA in
   each checkout; do not deploy a branch just because its name matches.
2. Back up Stalwart, Mailer, and Messenger before changing shared services.
   Leave Stalwart running on the existing mail transport network. Verify 25,
   465, 587 and 993, its private management listener, TLS certificate, DNS,
   PTR and outbound SMTP reachability. Do not replace its mail volumes.
3. Apply the shared Stalwart AUTH and MAIL FROM sender policy documented in
   Mailer's `STALWART_DOMAIN_PROVISIONING.md`. Use separate Mailer and CS Mail
   API keys and SMTP identities. Confirm an authenticated Mailer sender cannot
   use a CS Mail mailbox address and the CS Mail relay cannot use a Mailer
   bounce address. Audit pre-existing Mailer domain IDs before enabling new
   customers.
4. Validate and deploy Mailer with its own `sh manage preflight` and
   `sh manage deploy`. Its frontend stays behind its Cloudflare Tunnel and its
   API/worker keep their existing database and queue. Run the Mailer public
   health check and a verified-domain send test.
5. Issue the CS Mail web certificate with DNS-01 and stage the independent
   platform edge on loopback ports using `../edge/README.md`. Test `mail` TLS,
   `dm`, and root/`www` through the staged edge. A 502 for `mail` is expected
   before CS Mail is deployed. The CS Mail backend network does not exist yet.
6. Recreate Messenger Nginx from the new production Compose without public
   port bindings and publish the edge on 80/443. Perform this short port
   handoff in a maintenance window and keep the documented rollback ready.
7. Set CS Mail's proxy mode to `edge`, deploy it for the first time, and run
   the full `GO_LIVE.md` and `certify-launch.sh` gates. Validate HTTPS for
   `mail.crescentsphere.com`, Messenger through the edge, and Mailer through
   its Tunnel. Keep public admission controls closed. Cut over a business
   domain's MX only after its mailbox, DNS and
   external send/receive tests pass. Do not turn on open signup or instant
   paid-plan activation during the test.

The platform edge is an independent Compose project, even though its source
files currently live in this repository. Updates to Messenger, Mailer or CS
Mail must not reclaim public 80/443 or replace Stalwart's shared mail data.
The root website can later become the company platform homepage by changing
only the root/`www` route in the edge plan; `dm`, `mail` and `mailer` retain
their separate hostnames.
