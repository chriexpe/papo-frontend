CREATE TABLE expression_favorites (
 server_id BLOB NOT NULL,user_id BLOB NOT NULL,kind TEXT NOT NULL,id TEXT NOT NULL,payload BLOB NOT NULL,used_order INTEGER NOT NULL,
 PRIMARY KEY(server_id,user_id,kind,id),FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id)
);
CREATE TABLE expression_recent (
 server_id BLOB NOT NULL,user_id BLOB NOT NULL,id TEXT NOT NULL,payload BLOB NOT NULL,used_order INTEGER NOT NULL,
 PRIMARY KEY(server_id,user_id,id),FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id)
);
CREATE TABLE expression_usage (
 server_id BLOB NOT NULL,user_id BLOB NOT NULL,id TEXT NOT NULL,count INTEGER NOT NULL,used_order INTEGER NOT NULL,
 PRIMARY KEY(server_id,user_id,id),FOREIGN KEY(server_id,user_id) REFERENCES accounts(server_id,user_id)
);
CREATE INDEX expression_favorites_order ON expression_favorites(used_order);
CREATE INDEX expression_recent_order ON expression_recent(used_order);
CREATE INDEX expression_usage_order ON expression_usage(used_order);
