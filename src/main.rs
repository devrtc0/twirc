#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

use std::{error, io};
use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use futures::{future::FutureExt, StreamExt};
use tokio::select;
use tokio::time::{self, Duration, Instant};
use tui::{Frame, layout::Corner, style::{Color, Modifier, Style}, Terminal, text::{Span, Spans}, widgets::{Block, Borders, List, ListItem}};
use tui::backend::{Backend, CrosstermBackend};
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::widgets::{Row, Table};
use twitch_irc::{ClientConfig, SecureWSTransport, TwitchIRCClient};
use twitch_irc::login::StaticLoginCredentials;
use twitch_irc::message::{ClearChatAction, RGBColor, ServerMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    let config = ClientConfig::default();
    let (mut incoming_messages, client) =
        TwitchIRCClient::<SecureWSTransport, StaticLoginCredentials>::new(config);

    client.join("krylia_sovetov".to_owned()).unwrap();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut reader = EventStream::new();
    loop {
        let event = reader.next().fuse();
        select! {
            ev = event => {
                match ev {
                    Some(Ok(event)) => {
                        match event {
                            Event::Key(ev) if ev.code == KeyCode::Char('q') => {
                                break;
                            }
                            // TODO probably it's worth doing it by timer
                            Event::Resize(_, _) => {
                                terminal.draw(ui)?;
                            }
                            _ => {},
                        }
                    },
                    None | Some(Err(_)) => {
                        break;
                    },
                }
            },
            Some(message) = incoming_messages.recv() => {
                match message {
                    ServerMessage::ClearChat(msg) => {
                        match msg.action {
                            ClearChatAction::ChatCleared => {
                            }
                            ClearChatAction::UserBanned{user_login: _, user_id} => {
                            }
                            ClearChatAction::UserTimedOut {user_id, user_login: _, timeout_length: _ } => {
                            }
                        }
                        terminal.draw(ui)?;
                    }
                    ServerMessage::ClearMsg(msg) => {
                        terminal.draw(ui)?;
                    }
                    ServerMessage::Privmsg(msg) => {
                        let message = Message {
                            message_id: msg.message_id,
                            sender_name: msg.sender.name,
                            sender_id: msg.sender.id,
                            message_text: msg.message_text,
                            sender_color: msg.name_color,
                            server_timestamp: msg.server_timestamp,
                        };
                        terminal.draw(ui)?;
                    }
                    _ => {}
                }
            },
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui<B: Backend>(frame: &mut Frame<B>) {}

#[derive(Debug, Clone)]
struct Verticals {
    pub chunks: Vec<Rect>,
    pub constraints: Vec<Constraint>,
}

impl Verticals {
    pub fn new(chunks: Vec<Rect>, constraints: Vec<Constraint>) -> Self {
        Self {
            chunks,
            constraints,
        }
    }
}

struct Message {
    message_id: String,
    sender_name: String,
    sender_id: String,
    message_text: String,
    server_timestamp: DateTime<Utc>,
    sender_color: Option<RGBColor>,
}
