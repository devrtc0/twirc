#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

use std::{error, io};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Stdout, StdoutLock};

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
use textwrap::Options;
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

    client.join("kochevnik".to_owned()).unwrap();

    let mut app = App::new();
    app.draw()?;

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
                            // TODO probably it's worth doing it with timer
                            Event::Resize(_, _) => {
                                app.draw()?;
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
                            ClearChatAction::UserBanned{user_login: _, user_id} => {
                                app.clear_user_messages(user_id);
                            }
                            ClearChatAction::UserTimedOut {user_id, user_login: _, timeout_length: _ } => {
                                app.clear_user_messages(user_id);
                            }
                            _ => {}
                        }
                        app.draw()?;
                    }
                    ServerMessage::ClearMsg(msg) => {
                        app.clear_message(msg.message_id);
                        app.draw()?;
                    }
                    ServerMessage::Privmsg(msg) => {
                        let message = Message {
                            message_id: msg.message_id,
                            sender_name: msg.sender.name,
                            sender_id: msg.sender.id,
                            message_text: msg.message_text,
                            sender_color: msg.name_color,
                            server_timestamp: msg.server_timestamp,
                            deleted: false,
                        };
                        app.add_message(message);
                        app.draw()?;
                    }
                    _ => {}
                }
            },
        }
    }

    Ok(())
}

struct App {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    messages: VecDeque<Message>,
}

impl App {
    fn new() -> Self {
        enable_raw_mode().unwrap();
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).unwrap();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.hide_cursor().unwrap();

        Self {
            terminal,
            messages: VecDeque::new(),
        }
    }

    fn clear_user_messages(&mut self, user_id: String) {
        for msg in &mut self.messages {
            if msg.sender_id == user_id {
                msg.deleted = true;
                break;
            }
        }
    }

    fn clear_message(&mut self, msg_id: String) {
        for msg in &mut self.messages {
            if msg.message_id == msg_id {
                msg.deleted = true;
                break;
            }
        }
    }

    fn add_message(&mut self, msg: Message) {
        if self.messages.len() > 30 {
            let removed = self.messages.pop_back().unwrap();
        }
        self.messages.push_front(msg);
    }

    fn draw(&mut self) -> Result<(), Box<dyn error::Error>> {
        self.terminal.draw(|f| {
            let list_item_list: Vec<_> = self.messages.iter()
                .map(|msg| {
                    let sender_name_color = if msg.deleted {
                        Color::Reset
                    } else {
                        msg.sender_color.as_ref()
                            .map(|c| Color::Rgb(c.r, c.g, c.b))
                            .unwrap_or(Color::Reset)
                    };
                    let header = Spans::from(vec![
                        Span::styled(msg.server_timestamp.format("%H:%M:%S").to_string(), Style::default().fg(Color::Blue)),
                        Span::raw(" "),
                        Span::styled(
                            msg.sender_name.to_owned(),
                            Style::default().fg(sender_name_color).add_modifier(Modifier::ITALIC).add_modifier(Modifier::BOLD),
                        ),
                    ]);
                    let width = f.size().width as usize - 2;
                    let options = Options::new(width).break_words(false);
                    let message_lines: Vec<_> = textwrap::wrap(&msg.message_text, options).iter()
                        .map(|line| match line {
                            Cow::Borrowed(txt) => *txt,
                            Cow::Owned(txt) => txt,
                        })
                        .map(|line| {
                            Spans::from(vec![Span::raw(line.to_owned())])
                        })
                        .collect();
                    let mut spans = vec![Spans::from("-".repeat(width)), header];
                    spans.extend(message_lines);
                    let block_color = if msg.deleted { Color::LightRed } else { Color::Reset };
                    ListItem::new(spans).style(Style::default().bg(block_color))
                })
                .collect();

            let msg_list = List::new(list_item_list)
                .block(Block::default().borders(Borders::ALL).title("Chat"))
                .start_corner(Corner::TopLeft);
            f.render_widget(msg_list, f.size());
        })?;

        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        disable_raw_mode().unwrap();
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
        self.terminal.show_cursor().unwrap();
    }
}

struct Message {
    message_id: String,
    sender_name: String,
    sender_id: String,
    message_text: String,
    server_timestamp: DateTime<Utc>,
    sender_color: Option<RGBColor>,
    deleted: bool,
}
