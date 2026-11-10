use std::io::ErrorKind;
use std::net::{Ipv4Addr, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::client::Request;
use tungstenite::http::HeaderValue;
use tungstenite::{HandshakeError, Message, WebSocket};

use super::verdict::{FRAME_HEADER, FRAME_TAG};

const LOGIN_HEADER: &str = "Tailscale-User-Login";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);
const VISIBLE: &str = r#"{"t":"visible","v":true}"#;

pub enum Handshake {
    Upgraded(Box<WebSocket<TcpStream>>),
    Refused(u16),
}

#[derive(Default)]
pub struct Received {
    pub binary: Vec<Vec<u8>>,
    pub texts: usize,
}

fn request(port: u16, login: Option<&str>) -> Result<Request, String> {
    let mut request: Request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .map_err(|e| e.to_string())?;
    if let Some(login) = login {
        request.headers_mut().insert(
            LOGIN_HEADER,
            HeaderValue::from_str(login).map_err(|e| e.to_string())?,
        );
    }
    Ok(request)
}

pub fn handshake(port: u16, login: Option<&str>) -> Result<Handshake, String> {
    let tcp: TcpStream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .map_err(|e| format!("connect 127.0.0.1:{port}: {e}"))?;
    tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    match tungstenite::client(request(port, login)?, tcp) {
        Ok((socket, _)) => Ok(Handshake::Upgraded(Box::new(socket))),
        Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            Ok(Handshake::Refused(response.status().as_u16()))
        }
        Err(HandshakeError::Failure(error)) => Err(format!("handshake: {error}")),
        Err(HandshakeError::Interrupted(_)) => {
            Err(format!("handshake: no reply within {HANDSHAKE_TIMEOUT:?}"))
        }
    }
}

fn send(socket: &mut WebSocket<TcpStream>, text: &str) -> Result<(), String> {
    socket
        .send(Message::text(text))
        .map_err(|e| format!("send {text}: {e}"))
}

fn ack(payload: &[u8]) -> Option<String> {
    match (payload.first(), payload.get(1..5)) {
        (Some(&FRAME_TAG), Some(id)) if payload.len() >= FRAME_HEADER => {
            let id: [u8; 4] = id.try_into().ok()?;
            Some(format!(r#"{{"t":"ack","id":{}}}"#, u32::from_be_bytes(id)))
        }
        (None | Some(_), None | Some(_)) => None,
    }
}

fn timed_out(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

pub fn watch(
    mut socket: Box<WebSocket<TcpStream>>,
    (width, height): (u32, u32),
    watched: Duration,
) -> Result<Received, String> {
    socket
        .get_mut()
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    send(
        &mut socket,
        &format!(r#"{{"t":"view","w":{width},"h":{height}}}"#),
    )?;
    send(&mut socket, VISIBLE)?;
    let mut received: Received = Received::default();
    let start: Instant = Instant::now();
    while start.elapsed() < watched {
        match socket.read() {
            Ok(Message::Binary(payload)) => {
                if let Some(ack) = ack(&payload) {
                    send(&mut socket, &ack)?;
                }
                received.binary.push(payload.to_vec());
            }
            Ok(Message::Text(_)) => received.texts += 1,
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Ok(Message::Close(frame)) => {
                return Err(format!("server closed the socket: {frame:?}"));
            }
            Err(error) if timed_out(&error) => {}
            Err(error) => return Err(format!("read: {error}")),
        }
    }
    socket.close(None).map_err(|e| format!("close: {e}"))?;
    Ok(received)
}

#[cfg(test)]
mod tests;
