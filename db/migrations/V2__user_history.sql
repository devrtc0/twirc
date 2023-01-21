CREATE TABLE IF NOT EXISTS user_history (
    channel_id INT NOT NULL,
    user_id INT NOT NULL,
    server_timestamp TIMESTAMPTZ NOT NULL,
    channel_name VARCHAR(64) NOT NULL,
    user_login VARCHAR(64) NOT NULL,
    ban BOOLEAN NOT NULL DEFAULT false,
    timeout_duration BIGINT
);
