CREATE TABLE servers (
    id BLOB PRIMARY KEY CHECK(length(id)=16),
    canonical_address TEXT NOT NULL UNIQUE,
    position INTEGER NOT NULL,
    added_at_us INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0 CHECK(hidden IN (0,1))
);
CREATE TABLE accounts (
    credential_generation INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id BLOB NOT NULL REFERENCES servers(id),
    user_id BLOB NOT NULL CHECK(length(user_id)=16),
    display_name TEXT,
    avatar_ref TEXT,
    position INTEGER NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0,1)),
    created_at_us INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0 CHECK(hidden IN (0,1)),
    UNIQUE(server_id,user_id)
);
CREATE UNIQUE INDEX one_active_account ON accounts(server_id) WHERE active=1;
CREATE TABLE operations (
    local_id BLOB PRIMARY KEY CHECK(length(local_id)=16),
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    depends_on BLOB REFERENCES operations(local_id),
    state TEXT NOT NULL CHECK(state IN ('Queued','Sending','Sent','Failed','Unknown')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
    last_error BLOB,
    created_at_us INTEGER NOT NULL,
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
CREATE TABLE drafts (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    text TEXT NOT NULL,
    mentions BLOB NOT NULL,
    reply_to BLOB CHECK(reply_to IS NULL OR length(reply_to)=16),
    updated_at_us INTEGER NOT NULL,
    PRIMARY KEY(server_id,user_id,channel_id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
CREATE TABLE read_points (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    local_point BLOB,
    uploaded_point BLOB,
    PRIMARY KEY(server_id,user_id,channel_id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
CREATE TABLE notification_ledger (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    message_id BLOB NOT NULL CHECK(length(message_id)=16),
    delivered_at_us INTEGER NOT NULL,
    PRIMARY KEY(server_id,user_id,message_id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
CREATE TABLE removal_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id BLOB NOT NULL REFERENCES servers(id),
    user_id BLOB CHECK(user_id IS NULL OR length(user_id)=16),
    step INTEGER NOT NULL DEFAULT 0 CHECK(step BETWEEN 0 AND 5),
    failure INTEGER,
    UNIQUE(server_id,user_id)
);
CREATE UNIQUE INDEX one_server_removal ON removal_jobs(server_id) WHERE user_id IS NULL;
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
