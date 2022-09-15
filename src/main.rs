#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

use duration_string::DurationString;
use log::{info, warn, error, debug};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Stdout, StdoutLock};
use std::path::PathBuf;
use std::{error, io};

use chrono::{DateTime, Utc};

use clap::Parser;
use serde::Deserialize;
use tokio::time::{self, Duration, Instant};
use tokio::{select, signal};
use tokio_postgres::types::Type;
use tokio_postgres::{GenericClient, NoTls};
use twitch_irc::login::StaticLoginCredentials;
use twitch_irc::message::{ClearChatAction, RGBColor, ServerMessage};
use twitch_irc::{ClientConfig, SecureWSTransport, TwitchIRCClient};
use uuid::Uuid;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, value_parser, value_name = "FILE")]
    config: PathBuf,
}

#[derive(Debug, PartialEq, Deserialize)]
struct AppConfig {
    streamers: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    pretty_env_logger::init();

    let app_config: AppConfig = {
        let cli = Cli::parse();
        let config_path = cli.config;
        if !config_path.exists() {
            panic!("file not exists");
        }
        let f = File::open(config_path)?;
        serde_yaml::from_reader(f)?
    };

    let (db_client, connection) = tokio_postgres::connect(
        "host=localhost port=5432 user=twirc password=twirc dbname=twirc",
        NoTls,
    )
    .await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            error!("connection error: {}", e);
        }
    });

    let config = ClientConfig::default();
    let (mut incoming_messages, twitch_client) = TwitchIRCClient::<SecureWSTransport, StaticLoginCredentials>::new(config);

    for streamer in app_config.streamers {
        twitch_client.join(streamer)?;
    }

    let insert_stmt = db_client.prepare_typed(r#"
    INSERT INTO twirc (channel_id, channel_login, sender_id, sender_login, sender_name, message_id, message_text, server_timestamp)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    "#, &[Type::INT4, Type::VARCHAR, Type::INT4, Type::VARCHAR, Type::VARCHAR, Type::UUID, Type::VARCHAR, Type::TIMESTAMPTZ]
    ).await?;
    let delete_stmt = db_client
        .prepare_typed(
            r#"
    UPDATE twirc SET deleted = true WHERE message_id = $1
    "#,
            &[Type::UUID],
        )
        .await?;

    let ban_stmt = db_client
        .prepare_typed(
            r#"
    UPDATE twirc SET deleted = true WHERE sender_id = $1 AND channel_id = $2
    "#,
            &[Type::INT4, Type::INT4],
        )
        .await?;

    loop {
        select! {
            _ = signal::ctrl_c() => {
                break;
            },
            Some(message) = incoming_messages.recv() => {
                match message {
                    ServerMessage::ClearChat(msg) => {
                        match msg.action {
                            ClearChatAction::UserBanned{user_login, user_id} => {
                                let channel_id: i32 = msg.channel_id.parse()?;
                                let sender_id: i32 = user_id.parse()?;
                                let count = db_client.execute(&ban_stmt,
                                    &[&sender_id, &channel_id]
                                ).await?;
                                if count == 0 {
                                    error!("No message was marked as deleted for user {} in channel {}", user_login, msg.channel_login);
                                } else {
                                    debug!("{} banned in channel({})", user_login, msg.channel_login);
                                }
                            }
                            ClearChatAction::UserTimedOut {user_id, user_login, timeout_length } => {
                                let channel_id: i32 = msg.channel_id.parse()?;
                                let sender_id: i32 = user_id.parse()?;
                                let count = db_client.execute(&ban_stmt,
                                    &[&sender_id, &channel_id]
                                ).await?;
                                if count == 0 {
                                    error!("No message was marked as deleted for user {} in channel {}", user_login, msg.channel_login);
                                } else {
                                    let duration = DurationString::from(timeout_length);
                                    debug!("{} timeouted in channel({}) for {}", user_login, msg.channel_login, duration);
                                }
                            }
                            _ => {}
                        }
                    }
                    ServerMessage::ClearMsg(msg) => {
                        let msg_id = Uuid::parse_str(&msg.message_id)?;
                        let count = db_client.execute(&delete_stmt,
                            &[&msg_id]
                        ).await?;
                        if count == 0 {
                            error!("No message was marked as deleted for user {} in channel {}", msg.sender_login, msg.channel_login);
                        } else {
                            debug!("User's({}) message ({}) deleted in channel({})", msg.sender_login, msg.message_text, msg.channel_login);
                        }
                    }
                    ServerMessage::Privmsg(msg) => {
                        let sender = &msg.sender;
                        let channel_id: i32 = msg.channel_id.parse()?;
                        let sender_id: i32 = sender.id.parse()?;
                        let msg_id = Uuid::parse_str(&msg.message_id)?;
                        let count = db_client.execute(&insert_stmt,
                            &[&channel_id, &msg.channel_login, &sender_id, &sender.login, &sender.name, &msg_id, &msg.message_text, &msg.server_timestamp]
                        ).await?;
                        debug!("({}): {}: '{}'", msg.channel_login, sender.name, msg.message_text);
                    }
                    _ => {}
                }
            },
        }
    }
    Ok(())
}
