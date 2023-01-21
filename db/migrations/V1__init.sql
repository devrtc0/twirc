CREATE TABLE IF NOT EXISTS message_history (
    message_id UUID NOT NULL,
    channel_id INT NOT NULL,
    sender_id INT NOT NULL,
    server_timestamp TIMESTAMPTZ NOT NULL,
    channel_login VARCHAR(64) NOT NULL,
    sender_login VARCHAR(64) NOT NULL,
    sender_name VARCHAR(64) NOT NULL,
    message_text VARCHAR(512) NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (message_id)
);

CREATE INDEX IF NOT EXISTS message_history_channel_sender_idx ON message_history (channel_id, sender_id);
