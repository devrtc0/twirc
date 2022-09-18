-- name: add_message!
-- param: channel_id: i32
-- param: channel_login: &str
-- param: sender_id: i32
-- param: sender_login: &str
-- param: sender_name: &str
-- param: message_id: &Uuid
-- param: message_text: &str
-- param: server_timestamp: &DateTime<Utc>
INSERT INTO message_history (channel_id, channel_login, sender_id, sender_login, sender_name, message_id, message_text, server_timestamp)
VALUES (:channel_id, :channel_login, :sender_id, :sender_login, :sender_name, :message_id, :message_text, :server_timestamp);

-- name: ban_user!
-- param: channel_id: i32 - Channel ID
-- param: user_id: i32 - User ID
-- param: server_timestamp: &DateTime<Utc>
-- param: channel_name: &str
-- param: user_login: &str
INSERT INTO user_history (channel_id, user_id, server_timestamp, channel_name, user_login, ban)
VALUES (:channel_id, :user_id, :server_timestamp, :channel_name, :user_login, true);

-- name: timeout_user!
-- param: channel_id: i32 - Channel ID
-- param: user_id: i32 - User ID
-- param: server_timestamp: &DateTime<Utc>
-- param: channel_name: &str
-- param: user_login: &str
-- param: timeout_duration: i64 - Timeout duration in seconds
INSERT INTO user_history (channel_id, user_id, server_timestamp, channel_name, user_login, timeout_duration)
VALUES (:channel_id, :user_id, :server_timestamp, :channel_name, :user_login, :timeout_duration);

-- name: delete_message!
-- param: message_id: &Uuid - Message ID
UPDATE message_history
SET deleted = true
WHERE message_id = :message_id;
