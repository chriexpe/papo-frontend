CREATE TABLE notification_state (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    settings BLOB NOT NULL DEFAULT x'7b7d',
    cursor BLOB,
    not_before_us INTEGER NOT NULL DEFAULT -9223372036854775808,
    PRIMARY KEY(server_id,user_id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
ALTER TABLE notification_ledger ADD COLUMN message_created_at_us INTEGER;
CREATE INDEX notification_ledger_age ON notification_ledger(server_id,user_id,message_created_at_us);
