use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::anyhow;
use axum::extract::ws::{Message, WebSocket};
use phonepad_core::decision::Decision;
use phonepad_core::desk::DeskInput;
use phonepad_core::geometry::{Point, Size};
use phonepad_core::input::InputCommand;
use phonepad_core::stream::{self, Stream, StreamCommand, StreamEvent, StreamInput};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task;

use crate::capture::{Frame, Frames};
use crate::clock::Clock;
use crate::desk::DeskMessage;
use crate::wire::{self, PhoneMessage};
use crate::{input, jpeg};

#[derive(Clone)]
pub struct SessionPorts {
    pub desk: mpsc::Sender<DeskMessage>,
    pub input: mpsc::Sender<InputCommand>,
    pub frames: Frames,
    pub cursor: watch::Receiver<Option<Point>>,
    pub view: watch::Sender<Option<Size>>,
    pub clock: Clock,
}

struct Session {
    socket: WebSocket,
    stream: Stream,
    frames: broadcast::Receiver<Arc<Frame>>,
    latest: Option<Arc<Frame>>,
    ports: SessionPorts,
}

pub async fn run(socket: WebSocket, ports: SessionPorts) {
    let mut session: Session = Session {
        socket,
        stream: Stream::Hidden,
        frames: ports.frames.subscribe(),
        latest: None,
        ports,
    };
    if let Err(err) = session.serve().await {
        eprintln!("session: {err}");
    }
    if let Err(err) = session.step(StreamInput::Visibility(false)).await {
        eprintln!("session close: {err:#}");
    }
    for release in input::release_buttons() {
        if session.ports.input.send(release).await.is_err() {
            eprintln!("session close: input thread gone");
        }
    }
}

impl Session {
    async fn serve(&mut self) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                incoming = self.socket.recv() => match incoming {
                    Some(Ok(Message::Text(text))) => self.on_text(text.as_str()).await?,
                    Some(Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Err(err)) => return Err(err.into()),
                },
                captured = self.frames.recv() => match captured {
                    Ok(frame) => {
                        self.latest = Some(frame);
                        self.step(StreamInput::FrameReady).await?;
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return Err(anyhow!("capture channel closed")),
                },
            }
        }
    }

    async fn on_text(&mut self, text: &str) -> anyhow::Result<()> {
        match wire::parse(text) {
            Ok(PhoneMessage::Input(command)) => {
                self.ports
                    .input
                    .send(command)
                    .await
                    .map_err(|_| anyhow!("input thread gone"))?;
                self.tell_desk(DeskInput::PhoneInput {
                    at_ms: self.ports.clock.now_ms(),
                })
                .await
            }
            Ok(PhoneMessage::Visible(visible)) => self.step(StreamInput::Visibility(visible)).await,
            Ok(PhoneMessage::View(size)) => {
                self.ports.view.send_replace(size);
                Ok(())
            }
            Ok(PhoneMessage::Ack) => self.step(StreamInput::Ack).await,
            Err(err) => {
                eprintln!("bad message {text:?}: {err}");
                Ok(())
            }
        }
    }

    async fn step(&mut self, input: StreamInput) -> anyhow::Result<()> {
        let mut inputs: VecDeque<StreamInput> = VecDeque::from([input]);
        while let Some(input) = inputs.pop_front() {
            let decision: Decision<StreamEvent, StreamCommand> =
                stream::decide(&self.stream, input);
            self.stream = decision.events.iter().fold(self.stream, stream::evolve);
            for command in decision.commands {
                inputs.extend(self.execute(command).await?);
            }
        }
        Ok(())
    }

    async fn execute(&mut self, command: StreamCommand) -> anyhow::Result<Option<StreamInput>> {
        match command {
            StreamCommand::JoinViewers => {
                self.tell_desk(DeskInput::ViewerJoined).await.map(|()| None)
            }
            StreamCommand::LeaveViewers => {
                self.tell_desk(DeskInput::ViewerLeft).await.map(|()| None)
            }
            StreamCommand::SendLatestFrame => self.send_latest_frame().await,
        }
    }

    async fn tell_desk(&mut self, input: DeskInput) -> anyhow::Result<()> {
        self.ports
            .desk
            .send(DeskMessage::Input(input))
            .await
            .map_err(|_| anyhow!("desk stopped"))
    }

    async fn send_latest_frame(&mut self) -> anyhow::Result<Option<StreamInput>> {
        let Some(frame) = self.latest.clone() else {
            eprintln!("session: no frame to send; settling the send as acknowledged");
            return Ok(Some(StreamInput::Ack));
        };
        let origin: Point = frame.origin;
        let jpeg: Vec<u8> = task::spawn_blocking(move || jpeg::encode(&frame)).await??;
        let Some(cursor) = *self.ports.cursor.borrow() else {
            eprintln!("session: no cursor sample; settling the send as acknowledged");
            return Ok(Some(StreamInput::Ack));
        };
        self.socket
            .send(Message::Binary(
                wire::frame_message(origin, cursor, &jpeg).into(),
            ))
            .await?;
        Ok(None)
    }
}
