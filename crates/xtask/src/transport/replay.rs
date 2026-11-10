use crate::latency::stats::{self, Spread};
use futures_util::{SinkExt, StreamExt};
use phonepad_core::decision::Decision;
use phonepad_core::stream::{
    self, Offer, Sent, Stream, StreamCommand, StreamEvent, StreamInput, UnitKind,
};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use wtransport::{ClientConfig, Connection, Endpoint, Identity, ServerConfig};

pub const TICK: Duration = Duration::from_millis(20);
pub const KEYFRAME_BYTES: u32 = 40_000;
pub const KEYFRAME_SPACING: Duration = Duration::from_millis(500);
pub const QUIC_PAYLOAD: u16 = 1200;
const UNIT_TAG: u8 = 0;
const ECHO_TAG: u8 = 1;
const ACK_TAG: u8 = 2;
const PING_TAG: u8 = 3;
const UNIT_HEADER: usize = 17;
const PING_BYTES: usize = 12;
const CONTROL_ROLE: u8 = b'c';
const PING_ROLE: u8 = b'p';
const MAX_UNI_STREAMS: u32 = 1024;
const IDLE: Duration = Duration::from_secs(10);
const DRAIN: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    WebSocket,
    WebTransport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Congestion {
    Cubic,
    Bbr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs {
    pub deltas: (u32, u32),
    pub congestion: Congestion,
}

mod measure;
mod sender;
mod viewer;
mod wire;
mod ws;
mod wt;

pub use measure::*;
pub use sender::*;
pub use viewer::*;
pub use wire::*;
pub use ws::*;
pub use wt::*;

pub enum Bound {
    WebSocket(TcpListener),
    WebTransport(WtServer),
}

pub async fn bind(
    transport: Transport,
    port: u16,
    congestion: Congestion,
) -> Result<(Bound, u16), String> {
    match transport {
        Transport::WebSocket => {
            let listener: TcpListener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
                .await
                .map_err(|e| format!("ws bind {port}: {e}"))?;
            let bound: u16 = listener.local_addr().map_err(|e| e.to_string())?.port();
            Ok((Bound::WebSocket(listener), bound))
        }
        Transport::WebTransport => {
            let server: WtServer = wt_bind(port, congestion)?;
            let bound: u16 = server.port;
            Ok((Bound::WebTransport(server), bound))
        }
    }
}

pub async fn run(
    bound: Bound,
    connect_port: u16,
    knobs: Knobs,
    (settle, measured): (Duration, Duration),
) -> Result<Outcome, String> {
    let epoch: Instant = Instant::now();
    let until: Instant = epoch + settle + measured + DRAIN;
    let (server, viewer): (Result<Log, String>, Result<Viewer, String>) = match bound {
        Bound::WebSocket(listener) => {
            tokio::join!(
                ws_server(listener, knobs.deltas, epoch, until),
                ws_viewer(connect_port, epoch, until)
            )
        }
        Bound::WebTransport(server) => {
            let hash: wtransport::tls::Sha256Digest = server.hash.clone();
            tokio::join!(
                wt_server(server, knobs.deltas, epoch, until),
                wt_viewer(connect_port, hash, knobs.congestion, epoch, until)
            )
        }
    };
    let log: Log = viewer?.log(server?);
    let from_us: u64 = u64::try_from(settle.as_micros()).unwrap_or(u64::MAX);
    let to_us: u64 = u64::try_from((settle + measured).as_micros()).unwrap_or(u64::MAX);
    Ok(outcome(&log, from_us, to_us))
}

pub async fn serve(server: WtServer, deltas: (u32, u32), lasting: Duration) -> Result<Log, String> {
    let epoch: Instant = Instant::now();
    wt_server(server, deltas, epoch, epoch + lasting).await
}

#[cfg(test)]
mod tests;
