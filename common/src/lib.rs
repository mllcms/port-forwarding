#![allow(dead_code)]
use tokio::net::TcpStream;

/// tcp 流量转发
pub async fn forward(mut from: TcpStream, mut to: TcpStream) {
    tokio::spawn(async move {
        let (mut from_reader, mut from_writer) = from.split();
        let (mut to_reader, mut to_writer) = to.split();

        let from_to = tokio::io::copy(&mut from_reader, &mut to_writer);
        let to_from = tokio::io::copy(&mut to_reader, &mut from_writer);

        tokio::select! {
            _ = from_to => {},
            _ = to_from => {},
        }
    });
}

#[derive(Debug)]
pub enum Message {
    Ping,
    Pong,
    New,
    Other(String),
}

impl ToString for Message {
    fn to_string(&self) -> String {
        match self {
            Message::Ping => "ping\n".to_string(),
            Message::Pong => "pong\n".to_string(),
            Message::New => "new\n".to_string(),
            Message::Other(s) => format!("{}\n", s),
        }
    }
}

impl From<&str> for Message {
    fn from(s: &str) -> Self {
        match s {
            "ping\n" => Message::Ping,
            "pong\n" => Message::Pong,
            "new\n" => Message::New,
            other => Message::Other(other.to_string()),
        }
    }
}
