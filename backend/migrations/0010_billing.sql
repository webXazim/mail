-- WS4: manual-payment billing. Plans are admin-editable rows (not code) so the
-- entitlements enforced across the API live in one table; customers order a
-- plan and pay out-of-band (bank/PayPal), then an admin approves the order to
-- activate it. No external processor is involved.

CREATE TABLE plans (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    price_cents INTEGER NOT NULL DEFAULT 0 CHECK (price_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'USD',
    interval TEXT NOT NULL DEFAULT 'month' CHECK (interval IN ('month', 'year')),
    mailbox_bytes BIGINT NOT NULL CHECK (mailbox_bytes > 0),
    max_attachment_bytes BIGINT NOT NULL CHECK (max_attachment_bytes > 0),
    max_recipients INTEGER NOT NULL CHECK (max_recipients > 0),
    daily_send_limit INTEGER NOT NULL CHECK (daily_send_limit >= 0),
    seats INTEGER NOT NULL DEFAULT 1 CHECK (seats > 0),
    features JSONB NOT NULL DEFAULT '[]'::jsonb,
    sort_order INTEGER NOT NULL DEFAULT 0,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO plans
    (code, name, price_cents, mailbox_bytes, max_attachment_bytes, max_recipients,
     daily_send_limit, seats, features, sort_order)
VALUES
    ('solo', 'Harbor Solo', 0,
     5368709120, 26214400, 25, 300, 1,
     '["1 mailbox","5 GB storage","IMAP, SMTP, JMAP"]'::jsonb, 1),
    ('team', 'Harbor Team', 800,
     53687091200, 52428800, 50, 2000, 5,
     '["5 mailboxes","50 GB storage","Shared aliases","Priority support"]'::jsonb, 2),
    ('business', 'Harbor Business', 1600,
     214748364800, 104857600, 100, 10000, 25,
     '["25 mailboxes","200 GB storage","Admin & audit","99.9% uptime target"]'::jsonb, 3);

-- Singleton row with the out-of-band payment instructions shown to customers.
CREATE TABLE billing_settings (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    bank_details TEXT NOT NULL DEFAULT '',
    paypal_email TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO billing_settings (id) VALUES (TRUE);

CREATE SEQUENCE invoice_number_seq;

-- An order doubles as the invoice: it snapshots plan pricing at order time and
-- receives an invoice number when an admin marks it paid.
CREATE TABLE orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    plan_code TEXT NOT NULL REFERENCES plans(code),
    plan_name TEXT NOT NULL,
    amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'USD',
    interval TEXT NOT NULL DEFAULT 'month',
    seats INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'submitted', 'paid', 'cancelled', 'rejected')),
    payment_method TEXT NOT NULL DEFAULT 'bank'
        CHECK (payment_method IN ('bank', 'paypal', 'card', 'other')),
    payment_reference TEXT NOT NULL DEFAULT '',
    customer_note TEXT NOT NULL DEFAULT '',
    admin_note TEXT NOT NULL DEFAULT '',
    invoice_number TEXT UNIQUE,
    reviewed_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    submitted_at TIMESTAMPTZ,
    reviewed_at TIMESTAMPTZ,
    paid_at TIMESTAMPTZ,
    activated_at TIMESTAMPTZ
);

CREATE INDEX orders_user_idx ON orders (user_id, created_at DESC);
CREATE INDEX orders_status_idx ON orders (status, created_at DESC);
