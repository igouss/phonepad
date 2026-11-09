use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::anyhow;
use axum::extract::ws::{Message, WebSocket};
use phonepad_core::decision::Decision;
use phonepad_core::desk::DeskInput;
use phonepad_core::geometry::{Point, Size};
use phonepad_core::input::InputCommand;
use phonepad_core::stream::{self, Stream, StreamCommand, StreamEvent, StreamInput};
use tokio::sync::{mpsc, watch};
use tokio::task;

use crate::capture::{Frame, LatestFrame};
use crate::clock::Clock;
use crate::desk::DeskMessage;
use crate::wire::{self, PhoneMessage};
use crate::{input, jpeg};

#[derive(Clone)]
pub struct SessionPorts {
    pub desk: mpsc::Sender<DeskMessage>,
    pub input: mpsc::Sender<InputCommand>,
    pub frames: watch::Receiver<LatestFrame>,
    pub cursor: watch::Receiver<Option<Point>>,
    pub view: watch::Sender<Option<Size>>,
    pub clock: Clock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    region: Point,
    cursor: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Update {
    Image,
    CursorOnly { region: Point, cursor: Point },
    Nothing,
}

fn next_update(image_waiting: bool, shown: Option<Shown>, cursor: Option<Point>) -> Update {
    match (cursor, image_waiting, shown) {
        (None, _, _) => Update::Nothing,
        (Some(_), true, _) => Update::Image,
        (Some(cursor), false, Some(shown)) if shown.cursor != cursor => Update::CursorOnly {
            region: shown.region,
            cursor,
        },
        (Some(_), false, Some(_) | None) => Update::Nothing,
    }
}

struct Session {
    socket: WebSocket,
    stream: Stream,
    frames: watch::Receiver<LatestFrame>,
    cursor: watch::Receiver<Option<Point>>,
    sent_image: LatestFrame,
    shown: Option<Shown>,
    ports: SessionPorts,
}

pub async fn run(socket: WebSocket, ports: SessionPorts) {
    let mut session: Session = Session {
        socket,
        stream: Stream::Hidden,
        frames: ports.frames.clone(),
        cursor: ports.cursor.clone(),
        sent_image: None,
        shown: None,
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
                changed = self.frames.changed() => {
                    changed?;
                    self.offer_update().await?;
                }
                changed = self.cursor.changed() => {
                    changed?;
                    self.offer_update().await?;
                }
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
            Ok(PhoneMessage::Visible(visible)) => {
                self.step(StreamInput::Visibility(visible)).await?;
                self.offer_update().await
            }
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

    fn update(&self) -> Update {
        let image_waiting: bool = match (self.frames.borrow().as_ref(), self.sent_image.as_ref()) {
            (Some(latest), Some(sent)) => !Arc::ptr_eq(latest, sent),
            (Some(_), None) => true,
            (None, Some(_) | None) => false,
        };
        next_update(image_waiting, self.shown, *self.cursor.borrow())
    }

    async fn offer_update(&mut self) -> anyhow::Result<()> {
        match self.update() {
            Update::Image | Update::CursorOnly { .. } => self.step(StreamInput::FrameReady).await,
            Update::Nothing => Ok(()),
        }
    }

    async fn send_latest_frame(&mut self) -> anyhow::Result<Option<StreamInput>> {
        match self.update() {
            Update::Image => self.send_image().await,
            Update::CursorOnly { region, cursor } => {
                self.socket
                    .send(Message::Binary(
                        wire::frame_message(region, cursor, &[]).into(),
                    ))
                    .await?;
                self.shown = Some(Shown { region, cursor });
                Ok(None)
            }
            Update::Nothing => Ok(Some(StreamInput::Ack)),
        }
    }

    async fn send_image(&mut self) -> anyhow::Result<Option<StreamInput>> {
        let Some(frame) = self.frames.borrow().clone() else {
            return Ok(Some(StreamInput::Ack));
        };
        let encoded: Arc<Frame> = Arc::clone(&frame);
        let jpeg: Vec<u8> = task::spawn_blocking(move || jpeg::encode(&encoded)).await??;
        let Some(cursor) = *self.cursor.borrow() else {
            return Ok(Some(StreamInput::Ack));
        };
        self.socket
            .send(Message::Binary(
                wire::frame_message(frame.origin, cursor, &jpeg).into(),
            ))
            .await?;
        self.shown = Some(Shown {
            region: frame.origin,
            cursor,
        });
        self.sent_image = Some(frame);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION: Point = Point { x: 100, y: 200 };
    const HERE: Point = Point { x: 400, y: 500 };
    const THERE: Point = Point { x: 401, y: 500 };

    fn shown_at(cursor: Point) -> Option<Shown> {
        Some(Shown {
            region: REGION,
            cursor,
        })
    }

    #[test]
    fn no_cursor_sample_sends_nothing() {
        assert_eq!(next_update(true, None, None), Update::Nothing);
    }

    #[test]
    fn nothing_shown_and_no_image_sends_nothing() {
        assert_eq!(next_update(false, None, Some(HERE)), Update::Nothing);
    }

    #[test]
    fn first_image_is_sent() {
        assert_eq!(next_update(true, None, Some(HERE)), Update::Image);
    }

    #[test]
    fn new_image_wins_over_cursor_move() {
        assert_eq!(
            next_update(true, shown_at(HERE), Some(THERE)),
            Update::Image
        );
    }

    #[test]
    fn cursor_move_without_new_image_sends_header_for_shown_region() {
        assert_eq!(
            next_update(false, shown_at(HERE), Some(THERE)),
            Update::CursorOnly {
                region: REGION,
                cursor: THERE
            }
        );
    }

    #[test]
    fn still_cursor_without_new_image_sends_nothing() {
        assert_eq!(
            next_update(false, shown_at(HERE), Some(HERE)),
            Update::Nothing
        );
    }
}
