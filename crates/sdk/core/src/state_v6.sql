CREATE TABLE emoji_catalog (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    id BLOB NOT NULL CHECK(length(id)=16),
    name TEXT NOT NULL,
    format TEXT NOT NULL,
    created_at TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY(server_id,user_id,id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
