#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

use async_trait::async_trait;
#[cfg(feature = "mdbx")]
use serde::{Serialize, Deserialize};
use core::panic;
use std::ops::Add;
#[cfg(feature = "pg")]
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use duration_string::DurationString;
use log::{debug, error, info, warn};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Stdout, StdoutLock};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{error, fs, io};
use std::env::temp_dir;
use tokio::sync::watch::{self, Receiver};

use chrono::{DateTime, Utc};
use clap::builder::Str;

use clap::Parser;
#[cfg(feature = "pg")]
use include_postgres_sql::*;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::{self, Duration, Instant};
use tokio::{select, signal, task};
#[cfg(feature = "pg")]
use tokio_postgres::{types::Type, Config, GenericClient, NoTls};
use twitch_irc::login::StaticLoginCredentials;
use twitch_irc::message::{ClearChatAction, RGBColor, ServerMessage};
use twitch_irc::{ClientConfig, SecureWSTransport, TwitchIRCClient};

#[cfg(feature = "pg")]
include_sql!("db/scripts/library.sql");

#[cfg(feature = "mdbx")]
type Database = libmdbx::Database<libmdbx::NoWriteMap>;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    streamers: Vec<String>,
}

async fn watch_task(
    streamer: String,
    mut rx: Receiver<()>,
    store_msg_sender: StoreMsgSender,
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
                                store_msg_sender.ban_user(BanMessage {
                                    channel_id: msg.channel_id.parse()?,
                                    channel_login: msg.channel_login.to_owned(),
                                    user_id: user_id.parse()?,
                                    user_login: user_login.to_owned(),
                                    server_timestamp: msg.server_timestamp,
                                }).await;

                                warn!("{user_login} banned in channel({})", msg.channel_login);
                            }
                            ClearChatAction::UserTimedOut {user_id, user_login, timeout_length } => {
                                store_msg_sender.suspend_user(SuspendMessage {
                                    channel_id: msg.channel_id.parse()?,
                                    channel_login: msg.channel_login.to_owned(),
                                    user_id: user_id.parse()?,
                                    user_login: user_login.to_owned(),
                                    server_timestamp: msg.server_timestamp,
                                    timeout_duration: timeout_length,
                                }).await;

                                let duration = DurationString::from(timeout_length);
                                warn!("{user_login} timeouted in channel({}) for {duration}", msg.channel_login);
                            }
                            _ => {}
                        }
                    }
                    ServerMessage::ClearMsg(msg) => {
                        store_msg_sender.delete_message(DeleteMessage {
                            message_id: msg.message_id
                        }).await;

                        warn!("User's({}) message ({}) deleted in channel({})", msg.sender_login, msg.message_text, msg.channel_login);
                    }
                    ServerMessage::Privmsg(msg) => {
                        let sender = &msg.sender;
                        let channel_id: i32 = msg.channel_id.parse()?;
                        let sender_id: i32 = sender.id.parse()?;

                        store_msg_sender.add_message(AddMessage{
                            channel_id: channel_id,
                            sender_id: sender_id,
                            channel_login: msg.channel_login.to_owned(),
                            sender_login: sender.login.to_owned(),
                            sender_name: sender.name.to_owned(),
                            message_id: msg.message_id,
                            message_text: msg.message_text.to_owned(),
                            server_timestamp: msg.server_timestamp,
                        }).await;

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

    let store_msg_sender = new_store_sender();

    let mut handles = Vec::with_capacity(cli.streamers.len());
    let (tx, rx) = watch::channel(());
    for streamer in cli.streamers {
        let rx = rx.clone();
        let store_msg_sender = store_msg_sender.clone();
        let handle = tokio::spawn(async move {
            watch_task(streamer, rx, store_msg_sender).await.unwrap();
        });
        handles.push(handle);
    }

    signal::ctrl_c().await?;
    info!("Interrupted");
    tx.send(())?;
    store_msg_sender.stop().await;

    for handle in handles {
        handle.await?;
    }

    Ok(())
}

#[derive(Clone)]
struct StoreMsgSender {
    tx: Sender<StoreMessage>,
}

impl StoreMsgSender {
    fn new(tx: Sender<StoreMessage>) -> Self {
        Self { tx }
    }

    async fn add_message(&self, msg: AddMessage) {
        self.tx.send(StoreMessage::Add(msg)).await.unwrap();
    }

    async fn delete_message(&self, msg: DeleteMessage) {
        self.tx.send(StoreMessage::Delete(msg)).await.unwrap();
    }

    async fn ban_user(&self, msg: BanMessage) {
        self.tx.send(StoreMessage::Ban(msg)).await.unwrap();
    }

    async fn suspend_user(&self, msg: SuspendMessage) {
        self.tx.send(StoreMessage::Suspend(msg)).await.unwrap();
    }

    async fn stop(&self) {
        self.tx.send(StoreMessage::Break).await.unwrap();
    }
}

fn new_store_sender() -> StoreMsgSender {
    let (db_tx, db_rx) = mpsc::channel(200);
    new_store(db_rx);
    StoreMsgSender::new(db_tx)
}

#[cfg(not(any(feature = "mdbx", feature = "pg")))]
fn new_store(mut rx: mpsc::Receiver<StoreMessage>) -> JoinHandle<()> {
    task::spawn(async move {
        loop {
            match rx.recv().await {
                Some(StoreMessage::Break) => {
                    info!("DB loop break");
                    break;
                }
                _ => {}
            }
        }
    })
}

#[cfg(feature = "mdbx")]
fn new_store(mut rx: mpsc::Receiver<StoreMessage>) -> JoinHandle<()> {
    use libmdbx::{WriteFlags, TableFlags};

    let dir = temp_dir().join("twirc");
    if !dir.exists() {
        fs::create_dir(&dir).unwrap();
    }
    let db = Database::new().set_max_tables(10).open(&dir).unwrap();
    {
        let txn = db.begin_rw_txn().unwrap();
        let table = txn.create_table(Some("messages"), TableFlags::empty()).unwrap();
        let table = txn.create_table(Some("history"), TableFlags::empty()).unwrap();
        txn.commit().unwrap();
    }

    task::spawn_blocking(move || loop {
        match rx.blocking_recv() {
            Some(StoreMessage::Break) => {
                info!("DB loop break");
                break;
            }
            Some(StoreMessage::Add(msg)) => {
                let tx = db.begin_rw_txn().unwrap();
                let table = tx.open_table(Some("messages")).unwrap();
                let message_id = msg.message_id.clone();
                let record: ChatMessage = msg.into();
                let json = serde_json::to_vec(&record).unwrap();
                tx.put(&table, message_id, json, WriteFlags::empty()).unwrap();
                tx.commit().unwrap();
            }
            Some(StoreMessage::Delete(msg)) => {
                let tx = db.begin_rw_txn().unwrap();
                let table = tx.open_table(Some("messages")).unwrap();
                let raw: Vec<u8> = tx.get(&table, msg.message_id.as_bytes()).unwrap().unwrap();
                let mut record: ChatMessage = serde_json::from_slice(&raw).unwrap();
                record.deleted = true;

                let json = serde_json::to_vec(&record).unwrap();
                tx.put(&table, msg.message_id.as_bytes(), json, WriteFlags::empty()).unwrap();
                tx.commit().unwrap();
            }
            Some(StoreMessage::Ban(msg)) => {
                let tx = db.begin_rw_txn().unwrap();
                let table = tx.open_table(Some("history")).unwrap();
                let key = msg.server_timestamp.timestamp_nanos().to_ne_bytes();
                let record: ChatHistory = msg.into();
                let json = serde_json::to_string(&record).unwrap();
                tx.put(&table, key, json, WriteFlags::empty()).unwrap();
                tx.commit().unwrap();
            }
            Some(StoreMessage::Suspend(msg)) => {
                let tx = db.begin_rw_txn().unwrap();
                let table = tx.open_table(Some("history")).unwrap();
                let key = msg.server_timestamp.timestamp_nanos().to_ne_bytes();
                let record: ChatHistory = msg.into();
                let json = serde_json::to_string(&record).unwrap();
                tx.put(&table, key, json, WriteFlags::empty()).unwrap();
                tx.commit().unwrap();
            }
            _ => {}
        }
    })
}

#[cfg(feature = "pg")]
fn new_store(mut rx: mpsc::Receiver<StoreMessage>) -> JoinHandle<()> {
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
    let pool = Pool::builder(mgr).build().unwrap();

    task::spawn(async move {
        loop {
            match rx.recv().await {
                Some(StoreMessage::Break) => {
                    info!("DB loop break");
                    break;
                }
                Some(StoreMessage::Add(msg)) => {
                    let client = pool.get().await.unwrap();
                    client
                        .add_message(
                            msg.channel_id,
                            &msg.channel_login,
                            msg.sender_id,
                            &msg.sender_login,
                            &msg.sender_name,
                            &msg.message_id,
                            &msg.message_text,
                            &msg.server_timestamp,
                        )
                        .await
                        .unwrap();
                }
                Some(StoreMessage::Delete(msg)) => {
                    let client = pool.get().await.unwrap();
                    client.delete_message(&msg.message_id).await.unwrap();
                }
                Some(StoreMessage::Ban(msg)) => {
                    let client = pool.get().await.unwrap();
                    client
                        .ban_user(
                            msg.channel_id,
                            msg.user_id,
                            &msg.server_timestamp,
                            &msg.channel_login,
                            &msg.user_login,
                        )
                        .await
                        .unwrap();
                }
                Some(StoreMessage::Suspend(msg)) => {
                    let client = pool.get().await.unwrap();
                    client
                        .timeout_user(
                            msg.channel_id,
                            msg.user_id,
                            &msg.server_timestamp,
                            &msg.channel_login,
                            &msg.user_login,
                            msg.timeout_duration.as_secs() as i64,
                        )
                        .await
                        .unwrap();
                }
                _ => {}
            }
        }
    })
}

#[cfg(feature = "mdbx")]
#[derive(Serialize, Deserialize)]
struct ChatMessage {
    channel_id: i32,
    sender_id: i32,
    channel_login: String,
    sender_login: String,
    sender_name: String,
    message_text: String,
    server_timestamp: DateTime<Utc>,
    deleted: bool,
}
#[cfg(feature = "mdbx")]
impl From<AddMessage> for ChatMessage {
    fn from(value: AddMessage) -> Self {
        Self {
            channel_id: value.channel_id,
            sender_id: value.sender_id,
            channel_login: value.channel_login,
            sender_login: value.sender_login,
            sender_name: value.sender_name,
            message_text: value.message_text,
            server_timestamp: value.server_timestamp,
            deleted: false,
        }
    }
}
#[cfg(feature = "mdbx")]
#[derive(Serialize, Deserialize)]
struct ChatHistory {
    channel_id: i32,
    channel_login: String,
    user_id: i32,
    user_login: String,
    server_timestamp: DateTime<Utc>,
    timeout_duration: Duration,
}
#[cfg(feature = "mdbx")]
impl From<BanMessage> for ChatHistory {
    fn from(value: BanMessage) -> Self {
        Self {
            channel_id: value.channel_id,
            channel_login: value.channel_login,
            user_id: value.user_id,
            user_login: value.user_login,
            server_timestamp: value.server_timestamp,
            timeout_duration: Duration::MAX,
        }
    }
}
#[cfg(feature = "mdbx")]
impl From<SuspendMessage> for ChatHistory {
    fn from(value: SuspendMessage) -> Self {
        Self {
            channel_id: value.channel_id,
            channel_login: value.channel_login,
            user_id: value.user_id,
            user_login: value.user_login,
            server_timestamp: value.server_timestamp,
            timeout_duration: value.timeout_duration,
        }
    }
}

#[derive(Debug)]
struct AddMessage {
    channel_id: i32,
    sender_id: i32,
    channel_login: String,
    sender_login: String,
    sender_name: String,
    message_id: String,
    message_text: String,
    server_timestamp: DateTime<Utc>,
}

#[derive(Debug)]
struct DeleteMessage {
    message_id: String,
}

#[derive(Debug)]
struct BanMessage {
    channel_id: i32,
    channel_login: String,
    user_id: i32,
    user_login: String,
    server_timestamp: DateTime<Utc>,
}

#[derive(Debug)]
struct SuspendMessage {
    channel_id: i32,
    channel_login: String,
    user_id: i32,
    user_login: String,
    server_timestamp: DateTime<Utc>,
    timeout_duration: Duration,
}

#[derive(Debug)]
enum StoreMessage {
    Break,
    Add(AddMessage),
    Delete(DeleteMessage),
    Ban(BanMessage),
    Suspend(SuspendMessage),
}
