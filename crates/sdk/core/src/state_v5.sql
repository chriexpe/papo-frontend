CREATE TABLE catch_up_intervals (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    seen_to BLOB,
    start_after BLOB,
    end_before BLOB,
    covered_spans BLOB,
    PRIMARY KEY(server_id,user_id,channel_id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
CREATE TABLE saved_reading_positions (
    server_id BLOB NOT NULL,
    user_id BLOB NOT NULL,
    channel_id BLOB NOT NULL CHECK(length(channel_id)=16),
    message_id BLOB NOT NULL CHECK(length(message_id)=16),
    offset_units INTEGER NOT NULL,
    PRIMARY KEY(server_id,user_id,channel_id),
    FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id) ON DELETE CASCADE
);
