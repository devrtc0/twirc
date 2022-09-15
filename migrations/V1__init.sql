CREATE TABLE IF NOT EXISTS twirc (
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

CREATE INDEX IF NOT EXISTS twirc_channel_sender_idx ON twirc (channel_id, sender_id);
