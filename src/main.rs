#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

use duration_string::DurationString;
use log::{debug, error, info, warn};
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

static TASK_COUNTER: AtomicUsize = AtomicUsize::new(0);

async fn watch_task(
    streamers: Vec<String>,
    mut rx: Receiver<()>,
) -> Result<(), Box<dyn error::Error>> {
    let task_number = TASK_COUNTER.fetch_add(1, Ordering::SeqCst);
    info!("Initializing the task {}", task_number);
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
    let (mut incoming_messages, twitch_client) =
        TwitchIRCClient::<SecureWSTransport, StaticLoginCredentials>::new(config);

    for streamer in streamers {
        twitch_client.join(streamer).unwrap();
    }

    let insert_stmt = db_client
        .prepare_typed(
            include_str!("scripts/add_message.sql"),
            &[
                Type::INT4,
                Type::VARCHAR,
                Type::INT4,
                Type::VARCHAR,
                Type::VARCHAR,
                Type::UUID,
                Type::VARCHAR,
                Type::TIMESTAMPTZ,
            ],
        )
        .await?;

    let delete_stmt = db_client
        .prepare_typed(include_str!("scripts/delete_message.sql"), &[Type::UUID])
        .await?;

    let ban_stmt = db_client
        .prepare_typed(
            include_str!("scripts/ban_user.sql"),
            &[Type::INT4, Type::INT4],
        )
        .await?;

    loop {
        select! {
            _ = rx.changed() => {
                info!("Got an interrupt message in task {}", task_number);
                return Ok(());
            },
            Some(message) = incoming_messages.recv() => {
                match message {
                    ServerMessage::ClearChat(msg) => {
                        match msg.action {
                            ClearChatAction::UserBanned{user_login, user_id} => {
                                let channel_id: i32 = msg.channel_id.parse()?;
                                let sender_id: i32 = user_id.parse()?;
                                db_client.execute(&ban_stmt,
                                    &[&sender_id, &channel_id]
                                ).await?;
                                warn!("{} banned in channel({})", user_login, msg.channel_login);
                            }
                            ClearChatAction::UserTimedOut {user_id, user_login, timeout_length } => {
                                let channel_id: i32 = msg.channel_id.parse()?;
                                let sender_id: i32 = user_id.parse()?;
                                db_client.execute(&ban_stmt,
                                    &[&sender_id, &channel_id]
                                ).await?;
                                let duration = DurationString::from(timeout_length);
                                warn!("{} timeouted in channel({}) for {}", user_login, msg.channel_login, duration);
                            }
                            _ => {}
                        }
                    }
                    ServerMessage::ClearMsg(msg) => {
                        let msg_id = Uuid::parse_str(&msg.message_id)?;
                        db_client.execute(&delete_stmt,
                            &[&msg_id]
                        ).await?;
                        warn!("User's({}) message ({}) deleted in channel({})", msg.sender_login, msg.message_text, msg.channel_login);
                    }
                    ServerMessage::Privmsg(msg) => {
                        let sender = &msg.sender;
                        let channel_id: i32 = msg.channel_id.parse()?;
                        let sender_id: i32 = sender.id.parse()?;
                        let msg_id = Uuid::parse_str(&msg.message_id)?;
                        db_client.execute(&insert_stmt,
                            &[&channel_id, &msg.channel_login, &sender_id, &sender.login, &sender.name, &msg_id, &msg.message_text, &msg.server_timestamp]
                        ).await?;
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

    let app_config: AppConfig = {
        let cli = Cli::parse();
        let config_path = cli.config;
        if !config_path.exists() {
            panic!("file not exists");
        }
        let f = File::open(config_path)?;
        serde_yaml::from_reader(f)?
    };

    let cpu_nums = num_cpus::get();
    let mut handles = Vec::with_capacity(cpu_nums);
    let chunk_number = (app_config.streamers.len() + cpu_nums - 1) / cpu_nums;
    let chunks = app_config.streamers.chunks(chunk_number);
    let (tx, rx) = watch::channel(());
    for chunk in chunks {
        let streamers = chunk.into_iter().map(|x| x.clone()).collect();
        let _rx = rx.clone();
        let handle = tokio::spawn(async move {
            watch_task(streamers, _rx).await.unwrap();
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
