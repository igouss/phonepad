mod activity;
mod capture;
mod clock;
mod desk;
mod http;
mod hypr;
mod input;
mod keymap;
mod owe;
mod session;
mod timing;
mod tool;
mod wire;

use std::env;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use axum::serve::ListenerExt;
use phonepad_core::geometry::{Point, Size};
use phonepad_core::input::InputCommand;
use tokio::net::TcpListener;
use tokio::runtime::{self, Runtime};
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
const TIMINGS: &str = "PHONEPAD_TIMINGS";
const SHUTDOWN_BOUND: Duration = Duration::from_secs(3);

fn main() -> anyhow::Result<()> {
    let runtime: Runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
    let served: anyhow::Result<()> = runtime.block_on(serve());
    finish(runtime, SHUTDOWN_BOUND);
    served
}

fn finish(runtime: Runtime, bound: Duration) {
    runtime.shutdown_timeout(bound);
}

async fn serve() -> anyhow::Result<()> {
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
    let cursor: watch::Sender<Option<Point>> = watch::Sender::new(None);
    let feed: Feed = Feed {
        render_node: env::var_os("PHONEPAD_RENDER_NODE")
            .map_or_else(|| PathBuf::from(DEFAULT_RENDER_NODE), PathBuf::from),
        view: view_watch,
        cursor: cursor.clone(),
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
    let desk: mpsc::Sender<DeskMessage> = desk::spawn(pointer, feed);
    activity::spawn(desk.clone(), clock);
    let ports: SessionPorts = SessionPorts {
        desk: desk.clone(),
        input,
        units,
        keyframe,
        view,
        cursor,
        clock,
        timings: env::var_os(TIMINGS).is_some(),
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
    tokio::time::timeout(SHUTDOWN_BOUND, desk.send(DeskMessage::Shutdown(done)))
        .await
        .map_err(|_| anyhow!("desk inbox full for {SHUTDOWN_BOUND:?} at shutdown"))?
        .map_err(|_| anyhow!("desk stopped before shutdown"))?;
    tokio::time::timeout(SHUTDOWN_BOUND, finished)
        .await
        .map_err(|_| anyhow!("desk did not shut down within {SHUTDOWN_BOUND:?}"))?
        .context("desk shutdown")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;

    #[test]
    fn a_blocking_task_that_never_ends_cannot_hold_the_process_past_the_bound() {
        let runtime: Runtime = runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_keep, stuck): (mpsc::Sender<()>, mpsc::Receiver<()>) = mpsc::channel();
        runtime.spawn_blocking(move || stuck.recv());
        let started: Instant = Instant::now();
        finish(runtime, Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
