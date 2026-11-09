mod capture;
mod clock;
mod desk;
mod http;
mod hypr;
mod input;
mod owe;
mod session;
mod wire;

use std::env;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use axum::serve::ListenerExt;
use phonepad_core::geometry::Size;
use phonepad_core::input::InputCommand;
use tokio::net::TcpListener;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::capture::{Feed, KeyframeRequest, Unit};
use crate::clock::Clock;
use crate::desk::{DeskMessage, PointerStyle};
use crate::session::SessionPorts;

const BIG_POINTER: u32 = 64;
const DEFAULT_POINTER: u32 = 24;
const DEFAULT_RENDER_NODE: &str = "/dev/dri/renderD128";
const UNIT_BACKLOG: usize = 8;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let [_, port, login]: [String; 3] = args
        .try_into()
        .map_err(|_| anyhow!("usage: phonepad <port> <login>"))?;
    let port: u16 = port.parse().context("port")?;

    let clock: Clock = Clock::start();
    let input: mpsc::Sender<InputCommand> =
        input::spawn(input::open_device().context("create uinput device")?);
    let (view, view_watch): (watch::Sender<Option<Size>>, watch::Receiver<Option<Size>>) =
        watch::channel(None);
    let (units, _): (broadcast::Sender<Unit>, broadcast::Receiver<Unit>) =
        broadcast::channel(UNIT_BACKLOG);
    let keyframe: KeyframeRequest = KeyframeRequest::default();
    let feed: Feed = Feed {
        render_node: env::var_os("PHONEPAD_RENDER_NODE")
            .map_or_else(|| PathBuf::from(DEFAULT_RENDER_NODE), PathBuf::from),
        view: view_watch,
        cursor: watch::Sender::new(None),
        units: units.clone(),
        keyframe: keyframe.clone(),
    };
    let pointer: PointerStyle = PointerStyle {
        theme: desk::pointer_theme().await,
        normal: env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|size: String| size.parse().ok())
            .unwrap_or(DEFAULT_POINTER),
        big: BIG_POINTER,
    };
    let desk: mpsc::Sender<DeskMessage> = desk::spawn(pointer, clock, feed);
    let ports: SessionPorts = SessionPorts {
        desk: desk.clone(),
        input,
        units,
        keyframe,
        view,
        clock,
    };

    let listener: TcpListener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("bind 127.0.0.1:{port}"))?;
    let mut terminate: Signal = signal(SignalKind::terminate())?;
    let mut interrupt: Signal = signal(SignalKind::interrupt())?;
    println!("phonepad listening on http://127.0.0.1:{port}");
    let nodelay = listener.tap_io(|tcp: &mut tokio::net::TcpStream| {
        if let Err(err) = tcp.set_nodelay(true) {
            eprintln!("TCP_NODELAY: {err}");
        }
    });
    tokio::select! {
        served = axum::serve(nodelay, http::router(Arc::from(login), ports)).into_future() => served?,
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }

    let (done, finished): (oneshot::Sender<()>, oneshot::Receiver<()>) = oneshot::channel();
    desk.send(DeskMessage::Shutdown(done))
        .await
        .map_err(|_| anyhow!("desk stopped before shutdown"))?;
    finished.await.context("desk shutdown")?;
    Ok(())
}
