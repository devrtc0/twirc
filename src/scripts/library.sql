-- name: add_message!
-- param: channel_id: i32
-- param: channel_login: &str
-- param: sender_id: i32
-- param: sender_login: &str
-- param: sender_name: &str
-- param: message_id: &Uuid
-- param: message_text: &str
-- param: server_timestamp: &DateTime<Utc>
INSERT INTO twirc (channel_id, channel_login, sender_id, sender_login, sender_name, message_id, message_text, server_timestamp)
VALUES (:channel_id, :channel_login, :sender_id, :sender_login, :sender_name, :message_id, :message_text, :server_timestamp);

-- name: ban_user!
-- param: sender_id: i32 - Sender ID
-- param: channel_id: i32 - Channel ID
UPDATE twirc
SET deleted = true
WHERE sender_id = :sender_id AND channel_id = :channel_id;

-- name: delete_message!
-- param: message_id: &Uuid - Message ID
UPDATE twirc
SET deleted = true
WHERE message_id = :message_id;
