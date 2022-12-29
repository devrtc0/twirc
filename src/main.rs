#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use duration_string::DurationString;
use log::{debug, error, info, warn};
use core::panic;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Stdout, StdoutLock};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{error, io};
use tokio::sync::watch::{self, Receiver};

use chrono::{DateTime, Utc};

use clap::Parser;
use include_postgres_sql::*;
use tokio::time::{self, Duration, Instant};
use tokio::{select, signal};
use tokio_postgres::types::Type;
use tokio_postgres::{Config, GenericClient, NoTls};
use twitch_irc::login::StaticLoginCredentials;
use twitch_irc::message::{ClearChatAction, RGBColor, ServerMessage};
use twitch_irc::{ClientConfig, SecureWSTransport, TwitchIRCClient};
use uuid::Uuid;

include_sql!("src/scripts/library.sql");

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    streamers: Vec<String>,
}

async fn watch_task(
    streamer: String,
    mut rx: Receiver<()>,
    pool: Pool,
) -> Result<(), Box<dyn error::Error>> {
    info!("Initializing the task for {streamer}");

    let config = ClientConfig::default();
    let (mut incoming_messages, twitch_client) =
        TwitchIRCClient::<SecureWSTransport, StaticLoginCredentials>::new(config);

    twitch_client.join(streamer)?;

    loop {
        select! {
            _ = rx.changed() => {
                info!("Got an interrupt message");
                return Ok(());
            },
            Some(message) = incoming_messages.recv() => {
                match message {
                    ServerMessage::ClearChat(msg) => {
                        match msg.action {
                            ClearChatAction::UserBanned{user_login, user_id} => {
                                let channel_id: i32 = msg.channel_id.parse()?;
                                let user_id: i32 = user_id.parse()?;

                                let db_client = pool.get().await?;
                                db_client.ban_user(channel_id, user_id, &msg.server_timestamp, &msg.channel_login, &user_login).await?;

                                warn!("{user_login} banned in channel({})", msg.channel_login);
                            }
                            ClearChatAction::UserTimedOut {user_id, user_login, timeout_length } => {
                                let channel_id: i32 = msg.channel_id.parse()?;
                                let user_id: i32 = user_id.parse()?;

                                let db_client = pool.get().await?;
                                db_client.timeout_user(channel_id, user_id, &msg.server_timestamp, &msg.channel_login, &user_login, timeout_length.as_secs() as i64).await?;

                                let duration = DurationString::from(timeout_length);
                                warn!("{user_login} timeouted in channel({}) for {duration}", msg.channel_login);
                            }
                            _ => {}
                        }
                    }
                    ServerMessage::ClearMsg(msg) => {
                        let msg_id = Uuid::parse_str(&msg.message_id)?;

                        let db_client = pool.get().await?;
                        db_client.delete_message(&msg_id).await?;

                        warn!("User's({}) message ({}) deleted in channel({})", msg.sender_login, msg.message_text, msg.channel_login);
                    }
                    ServerMessage::Privmsg(msg) => {
                        let sender = &msg.sender;
                        let channel_id: i32 = msg.channel_id.parse()?;
                        let sender_id: i32 = sender.id.parse()?;
                        let msg_id = Uuid::parse_str(&msg.message_id)?;

                        let db_client = pool.get().await?;
                        db_client.add_message(channel_id, &msg.channel_login, sender_id, &sender.login, &sender.name, &msg_id, &msg.message_text, &msg.server_timestamp).await?;

                        info!("({}): {}: '{}'", msg.channel_login, sender.name, msg.message_text);
                    }
                    _ => {}
                }
            },
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    pretty_env_logger::init();

    let cli = Cli::parse();
    if cli.streamers.is_empty() {
        panic!("No streamers set");
    }
    info!("streamers: {:?}", cli.streamers);

    let pg_config = Config::new()
        .host("localhost")
        .port(5432)
        .user("twirc")
        .password("twirc")
        .dbname("twirc")
        .to_owned();
    let mgr_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
    let pool = Pool::builder(mgr).build()?;

    let mut handles = Vec::with_capacity(cli.streamers.len());
    let (tx, rx) = watch::channel(());
    for streamer in cli.streamers {
        let rx = rx.clone();
        let pool = pool.clone();
        let handle = tokio::spawn(async move {
            watch_task(streamer, rx, pool).await.unwrap();
        });
        handles.push(handle);
    }

    signal::ctrl_c().await?;
    info!("Interrupted");
    tx.send(())?;

    for handle in handles {
        handle.await?;
    }

    Ok(())
}
