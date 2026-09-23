# Root-domain application email

Mailer owns delivery for platform-generated email from the verified
`crescentsphere.com` domain. Each application needs a production Mailer API key
with `emails:send` scope in the workspace where that root domain is verified.
Use one key per application after the initial cutover. Never commit keys.

| Application | Private environment | Sender | Delivery path |
| --- | --- | --- | --- |
| CS Mail | `CS_MAILER_API_URL`, `CS_MAILER_API_KEY` | `mailer@crescentsphere.com` for platform messages | Mailer API; customer mailbox messages continue through Stalwart |
| Mailer | `ACCOUNT_EMAIL_API_KEY`, `ACCOUNT_EMAIL_FROM` | `mailer@crescentsphere.com` | Internal Mailer API |
| Notes | `CS_MAILER_API_URL`, `CS_MAILER_API_KEY`, `EMAIL_BACKEND`, `DEFAULT_FROM_EMAIL` | `notes@crescentsphere.com` | Mailer API from Django and the Rust delivery worker |
| Messenger | `CS_MAILER_API_URL`, `CS_MAILER_API_KEY`, `EMAIL_BACKEND`, `DEFAULT_FROM_EMAIL` | `messenger@crescentsphere.com` | Mailer API from Django |

For CS Mail, Notes, and Messenger, set
`CS_MAILER_API_URL=https://mailer.crescentsphere.com/api/v1/emails` in their
private production env files. Set the key in each private file with an editor;
do not put it on a command line or in Git. Notes and Messenger use
`EMAIL_BACKEND=config.mailer_email_backend.MailerEmailBackend`.

Mailer account email uses `ACCOUNT_EMAIL_FROM='CrescentSphere Mailer <mailer@crescentsphere.com>'`
and its existing `ACCOUNT_EMAIL_API_KEY`. Keep `AUTH_EMAIL_DELIVERY_ENABLED`
enabled only when the production account key is configured.

After deploying, send one login or verification message from each service,
one Notes document email, and one Messenger billing attachment. Confirm
`signed-by: crescentsphere.com` in a received message. Mailer must have object
storage configured for attachments. Keep the root-domain SPF, DKIM, DMARC, and
bounce records needed by the verified Mailer domain.
