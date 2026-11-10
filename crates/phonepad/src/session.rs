use anyhow::anyhow;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use phonepad_core::decision::Decision;
use phonepad_core::desk::DeskInput;
use phonepad_core::geometry::{Point, Size};
use phonepad_core::input::InputCommand;
use phonepad_core::stream::{self, Stream, StreamCommand, StreamEvent, StreamInput};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, watch};

use crate::capture::{KeyframeRequest, Unit};
use crate::clock::{self, Clock};
use crate::desk::DeskMessage;
use crate::input;
use crate::timing::{Drawn, Ledger, Record, Sent};
use crate::wire::{self, PhoneMessage};

#[derive(Clone)]
pub struct SessionPorts {
    pub desk: mpsc::Sender<DeskMessage>,
    pub input: mpsc::Sender<InputCommand>,
    pub units: broadcast::Sender<Unit>,
    pub keyframe: KeyframeRequest,
    pub view: watch::Sender<Option<Size>>,
    pub cursor: watch::Sender<Option<Point>>,
    pub clock: Clock,
    pub timings: bool,
}

struct Session {
    socket: WebSocket,
    stream: Stream,
    units: Option<broadcast::Receiver<Unit>>,
    cursor: Option<watch::Receiver<Option<Point>>>,
    ledger: Ledger,
    ports: SessionPorts,
}

pub async fn run(socket: WebSocket, ports: SessionPorts) {
    let mut session: Session = Session {
        socket,
        stream: Stream::Hidden,
        units: None,
        cursor: None,
        ledger: Ledger::default(),
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

async fn next_cursor(cursor: &mut Option<watch::Receiver<Option<Point>>>) -> Option<Point> {
    match cursor {
        Some(cursor) => match cursor.changed().await {
            Ok(()) => *cursor.borrow_and_update(),
            Err(_) => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Effect {
    TellDesk(DeskInput),
    Inject(InputCommand),
    Stream(StreamInput),
    SetView(Option<Size>),
    Acked(u32),
    Drawn(Drawn),
}

fn effects(message: PhoneMessage, at_ms: u64) -> Vec<Effect> {
    match message {
        PhoneMessage::Input(command) => vec![
            Effect::TellDesk(DeskInput::PhoneInput { at_ms }),
            Effect::Inject(command),
        ],
        PhoneMessage::Visible(visible) => vec![Effect::Stream(StreamInput::Visibility(visible))],
        PhoneMessage::View(size) => vec![Effect::SetView(size)],
        PhoneMessage::Ack(id) => vec![Effect::Acked(id), Effect::Stream(StreamInput::Ack)],
        PhoneMessage::Drawn(drawn) => vec![Effect::Drawn(drawn)],
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
                    Some(input) => self.step(input, received.as_ref().ok()).await?,
                    None => return Err(anyhow!("capture feed closed")),
                },
                Some(at) = next_cursor(&mut self.cursor) => self.send_cursor(at).await?,
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
            Effect::Acked(id) => {
                let record: Option<Record> = self.ledger.acked(id, clock::monotonic_ns());
                record.map_or(Ok(()), |record: Record| self.print(&record))
            }
            Effect::Drawn(drawn) => {
                let record: Record = self.ledger.drawn(drawn);
                self.print(&record)
            }
        }
    }

    async fn step(&mut self, input: StreamInput, unit: Option<&Unit>) -> anyhow::Result<()> {
        let decision: Decision<StreamEvent, StreamCommand> = stream::decide(&self.stream, input);
        self.stream = decision.events.iter().fold(self.stream, stream::evolve);
        for command in decision.commands {
            self.execute(command, unit).await?;
        }
        Ok(())
    }

    async fn execute(&mut self, command: StreamCommand, unit: Option<&Unit>) -> anyhow::Result<()> {
        match command {
            StreamCommand::JoinViewers => {
                self.units = Some(self.ports.units.subscribe());
                self.ledger = Ledger::default();
                let mut cursor: watch::Receiver<Option<Point>> = self.ports.cursor.subscribe();
                let current: Option<Point> = *cursor.borrow_and_update();
                self.cursor = Some(cursor);
                if let Some(at) = current {
                    self.send_cursor(at).await?;
                }
                self.tell_desk(DeskInput::ViewerJoined).await
            }
            StreamCommand::LeaveViewers => {
                self.units = None;
                self.cursor = None;
                self.tell_desk(DeskInput::ViewerLeft).await
            }
            StreamCommand::RequestKeyframe => {
                self.ports.keyframe.request();
                Ok(())
            }
            StreamCommand::SendUnit => {
                let unit: &Unit =
                    unit.ok_or_else(|| anyhow!("the stream sent a unit it was not offered"))?;
                self.ledger.sent(Sent {
                    id: unit.id,
                    kind: unit.kind,
                    bytes: unit.message.len(),
                    captured_ns: unit.captured_ns,
                    sent_ns: clock::monotonic_ns(),
                });
                self.socket
                    .send(Message::Binary(unit.message.clone()))
                    .await?;
                Ok(())
            }
        }
    }

    async fn send_cursor(&mut self, at: Point) -> anyhow::Result<()> {
        self.socket
            .send(Message::Binary(Bytes::from(wire::cursor_message(at))))
            .await?;
        Ok(())
    }

    fn print(&self, record: &Record) -> anyhow::Result<()> {
        if self.ports.timings {
            println!("timing {}", serde_json::to_string(record)?);
        }
        Ok(())
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
