use anyhow::anyhow;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use phonepad_core::decision::Decision;
use phonepad_core::desk::DeskInput;
use phonepad_core::geometry::Size;
use phonepad_core::input::InputCommand;
use phonepad_core::stream::{self, Stream, StreamCommand, StreamEvent, StreamInput};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};

use crate::capture::{KeyframeRequest, Unit};
use crate::clock::Clock;
use crate::desk::DeskMessage;
use crate::input;
use crate::wire::{self, PhoneMessage};

#[derive(Clone)]
pub struct SessionPorts {
    pub desk: mpsc::Sender<DeskMessage>,
    pub input: mpsc::Sender<InputCommand>,
    pub units: broadcast::Sender<Unit>,
    pub keyframe: KeyframeRequest,
    pub view: watch::Sender<Option<Size>>,
    pub clock: Clock,
}

struct Session {
    socket: WebSocket,
    stream: Stream,
    units: Option<broadcast::Receiver<Unit>>,
    ports: SessionPorts,
}

pub async fn run(socket: WebSocket, ports: SessionPorts) {
    let mut session: Session = Session {
        socket,
        stream: Stream::Hidden,
        units: None,
        ports,
    };
    if let Err(err) = session.serve().await {
        eprintln!("session: {err}");
    }
    if let Err(err) = session.step(StreamInput::Visibility(false), None).await {
        eprintln!("session close: {err:#}");
    }
    for release in input::release_buttons() {
        if session.ports.input.send(release).await.is_err() {
            eprintln!("session close: input thread gone");
        }
    }
}

async fn next_unit(units: &mut Option<broadcast::Receiver<Unit>>) -> Result<Unit, RecvError> {
    match units {
        Some(units) => units.recv().await,
        None => std::future::pending().await,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Effect {
    TellDesk(DeskInput),
    Inject(InputCommand),
    Stream(StreamInput),
    SetView(Option<Size>),
}

fn effects(message: PhoneMessage, at_ms: u64) -> Vec<Effect> {
    match message {
        PhoneMessage::Input(command) => vec![
            Effect::TellDesk(DeskInput::PhoneInput { at_ms }),
            Effect::Inject(command),
        ],
        PhoneMessage::Visible(visible) => vec![Effect::Stream(StreamInput::Visibility(visible))],
        PhoneMessage::View(size) => vec![Effect::SetView(size)],
        PhoneMessage::Ack => vec![Effect::Stream(StreamInput::Ack)],
        PhoneMessage::Key => vec![Effect::Stream(StreamInput::Missed)],
    }
}

fn unit_input(received: &Result<Unit, RecvError>) -> Option<StreamInput> {
    match received {
        Ok(unit) => Some(StreamInput::UnitReady(unit.kind)),
        Err(RecvError::Lagged(_)) => Some(StreamInput::Missed),
        Err(RecvError::Closed) => None,
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
                received = next_unit(&mut self.units) => match unit_input(&received) {
                    Some(input) => self.step(input, received.ok().map(|unit: Unit| unit.message)).await?,
                    None => return Err(anyhow!("capture feed closed")),
                },
            }
        }
    }

    async fn on_text(&mut self, text: &str) -> anyhow::Result<()> {
        match wire::parse(text) {
            Ok(message) => {
                for effect in effects(message, self.ports.clock.now_ms()) {
                    self.apply(effect).await?;
                }
                Ok(())
            }
            Err(err) => {
                eprintln!("bad message {text:?}: {err}");
                Ok(())
            }
        }
    }

    async fn apply(&mut self, effect: Effect) -> anyhow::Result<()> {
        match effect {
            Effect::TellDesk(input) => self.tell_desk(input).await,
            Effect::Inject(command) => self
                .ports
                .input
                .send(command)
                .await
                .map_err(|_| anyhow!("input thread gone")),
            Effect::Stream(input) => self.step(input, None).await,
            Effect::SetView(size) => {
                self.ports.view.send_replace(size);
                Ok(())
            }
        }
    }

    async fn step(&mut self, input: StreamInput, message: Option<Bytes>) -> anyhow::Result<()> {
        let decision: Decision<StreamEvent, StreamCommand> = stream::decide(&self.stream, input);
        self.stream = decision.events.iter().fold(self.stream, stream::evolve);
        for command in decision.commands {
            self.execute(command, message.as_ref()).await?;
        }
        Ok(())
    }

    async fn execute(
        &mut self,
        command: StreamCommand,
        message: Option<&Bytes>,
    ) -> anyhow::Result<()> {
        match command {
            StreamCommand::JoinViewers => {
                self.units = Some(self.ports.units.subscribe());
                self.tell_desk(DeskInput::ViewerJoined).await
            }
            StreamCommand::LeaveViewers => {
                self.units = None;
                self.tell_desk(DeskInput::ViewerLeft).await
            }
            StreamCommand::RequestKeyframe => {
                self.ports.keyframe.request();
                Ok(())
            }
            StreamCommand::SendUnit => {
                let message: Bytes = message
                    .cloned()
                    .ok_or_else(|| anyhow!("the stream sent a unit it was not offered"))?;
                self.socket.send(Message::Binary(message)).await?;
                Ok(())
            }
        }
    }

    async fn tell_desk(&mut self, input: DeskInput) -> anyhow::Result<()> {
        self.ports
            .desk
            .send(DeskMessage::Input(input))
            .await
            .map_err(|_| anyhow!("desk stopped"))
    }
}

#[cfg(test)]
mod tests;
