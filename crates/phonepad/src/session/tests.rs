use std::num::NonZeroU32;

use axum::body::Bytes;
use futures_util::{SinkExt, StreamExt};
use phonepad_core::geometry::Size;
use phonepad_core::input::Button;
use phonepad_core::stream::UnitKind;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::*;
use crate::capture::KeyframeRequest;

fn press() -> InputCommand {
    InputCommand::Button {
        button: Button::Left,
        pressed: true,
    }
}

#[test]
fn phone_input_reaches_the_desk_before_the_device() {
    assert_eq!(
        effects(PhoneMessage::Input(press()), 1234),
        vec![
            Effect::TellDesk(DeskInput::PhoneInput { at_ms: 1234 }),
            Effect::Inject(press()),
        ]
    );
}

#[test]
fn visibility_goes_to_the_stream() {
    assert_eq!(
        [
            effects(PhoneMessage::Visible(true), 0),
            effects(PhoneMessage::Visible(false), 0)
        ],
        [
            vec![Effect::Stream(StreamInput::Visibility(true))],
            vec![Effect::Stream(StreamInput::Visibility(false))]
        ]
    );
}

#[test]
fn view_size_is_published() {
    let size: Option<Size> = NonZeroU32::new(648)
        .zip(NonZeroU32::new(1072))
        .map(|(w, h): (NonZeroU32, NonZeroU32)| Size { w, h });
    assert_eq!(
        effects(PhoneMessage::View(size), 0),
        vec![Effect::SetView(size)]
    );
}

#[test]
fn an_ack_times_its_unit_and_credits_the_stream() {
    assert_eq!(effects(PhoneMessage::Ack(9), 0), vec![Effect::Acked(9)]);
}

#[test]
fn a_drawn_report_is_timed() {
    let drawn: Drawn = Drawn {
        id: 9,
        received_ms: 1.0,
        decoded_ms: 2.0,
        drawn_ms: 3.0,
    };
    assert_eq!(
        effects(PhoneMessage::Drawn(drawn), 0),
        vec![Effect::Drawn(drawn)]
    );
}

#[test]
fn keyframe_request_counts_as_a_missed_unit() {
    assert_eq!(
        effects(PhoneMessage::Key, 0),
        vec![Effect::Stream(StreamInput::Missed)]
    );
}

#[test]
fn a_received_unit_is_offered_with_its_kind_id_size_and_arrival() {
    let unit: Unit = Unit {
        kind: UnitKind::Keyframe,
        id: 7,
        captured_ns: 0,
        message: Bytes::from_static(b"au"),
    };
    assert_eq!(
        unit_input(&Ok(unit), 42),
        Some(StreamInput::UnitReady(Offer {
            kind: UnitKind::Keyframe,
            unit: stream::Sent {
                id: 7,
                bytes: 2,
                at_us: 42
            }
        }))
    );
}

#[test]
fn nanoseconds_become_whole_microseconds() {
    assert_eq!([micros(999), micros(1000), micros(2_500_999)], [0, 1, 2500]);
}

#[test]
fn a_lagged_feed_is_a_missed_unit() {
    assert_eq!(
        unit_input(&Err(RecvError::Lagged(3)), 0),
        Some(StreamInput::Missed)
    );
}

#[test]
fn a_closed_feed_ends_the_session() {
    assert_eq!(unit_input(&Err(RecvError::Closed), 0), None);
}

const LOGIN: &str = "owner@example.com";
const WAIT: std::time::Duration = std::time::Duration::from_secs(2);

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Wired {
    client: Client,
    desk: mpsc::Receiver<DeskMessage>,
    input: mpsc::Receiver<InputCommand>,
    units: broadcast::Sender<Unit>,
    view: watch::Receiver<Option<Size>>,
}

async fn wired() -> Wired {
    let (desk, desk_rx): (mpsc::Sender<DeskMessage>, mpsc::Receiver<DeskMessage>) =
        mpsc::channel(8);
    let (input, input_rx): (mpsc::Sender<InputCommand>, mpsc::Receiver<InputCommand>) =
        mpsc::channel(8);
    let (view, view_rx): (watch::Sender<Option<Size>>, watch::Receiver<Option<Size>>) =
        watch::channel(None);
    let (units, _): (broadcast::Sender<Unit>, broadcast::Receiver<Unit>) = broadcast::channel(4);
    let ports: SessionPorts = SessionPorts {
        desk,
        input,
        units: units.clone(),
        keyframe: KeyframeRequest::default(),
        view,
        cursor: watch::Sender::new(None),
        clock: Clock::start(),
        timings: false,
    };
    let listener: tokio::net::TcpListener =
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: std::net::SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(
        axum::serve(
            listener,
            crate::http::router(std::sync::Arc::from(LOGIN), ports),
        )
        .into_future(),
    );
    let mut request: axum::http::Request<()> =
        format!("ws://{address}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("tailscale-user-login", LOGIN.parse().unwrap());
    let (client, _): (Client, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    Wired {
        client,
        desk: desk_rx,
        input: input_rx,
        units,
        view: view_rx,
    }
}

async fn say(client: &mut Client, text: &str) {
    client
        .send(tokio_tungstenite::tungstenite::Message::text(text))
        .await
        .unwrap();
}

async fn desk_input(desk: &mut mpsc::Receiver<DeskMessage>) -> Option<DeskInput> {
    match tokio::time::timeout(WAIT, desk.recv()).await {
        Ok(Some(DeskMessage::Input(input))) => Some(input),
        Ok(Some(_) | None) | Err(_) => None,
    }
}

async fn injected(input: &mut mpsc::Receiver<InputCommand>) -> Option<InputCommand> {
    tokio::time::timeout(WAIT, input.recv())
        .await
        .ok()
        .flatten()
}

#[tokio::test]
async fn a_visible_phone_joins_the_viewers() {
    let mut wired: Wired = wired().await;
    say(&mut wired.client, r#"{"t":"visible","v":true}"#).await;
    assert_eq!(
        desk_input(&mut wired.desk).await,
        Some(DeskInput::ViewerJoined)
    );
}

#[tokio::test]
async fn a_move_is_announced_to_the_desk_then_injected() {
    let mut wired: Wired = wired().await;
    say(&mut wired.client, r#"{"t":"move","dx":3,"dy":-2}"#).await;
    assert_eq!(
        (
            matches!(
                desk_input(&mut wired.desk).await,
                Some(DeskInput::PhoneInput { .. })
            ),
            injected(&mut wired.input).await
        ),
        (true, Some(InputCommand::Move { dx: 3, dy: -2 }))
    );
}

#[tokio::test]
async fn a_view_size_is_published() {
    let mut wired: Wired = wired().await;
    say(&mut wired.client, r#"{"t":"view","w":648,"h":1072}"#).await;
    let changed: bool = tokio::time::timeout(WAIT, wired.view.changed())
        .await
        .is_ok_and(|changed: Result<(), watch::error::RecvError>| changed.is_ok());
    assert_eq!(
        (
            changed,
            wired
                .view
                .borrow()
                .map(|size: Size| (size.w.get(), size.h.get()))
        ),
        (true, Some((648, 1072)))
    );
}

#[tokio::test]
async fn a_keyframe_reaches_a_visible_phone() {
    let mut wired: Wired = wired().await;
    say(&mut wired.client, r#"{"t":"visible","v":true}"#).await;
    desk_input(&mut wired.desk).await;
    wired
        .units
        .send(Unit {
            kind: UnitKind::Keyframe,
            id: 0,
            captured_ns: 0,
            message: Bytes::from_static(b"keyframe"),
        })
        .unwrap();
    let received: Option<tokio_tungstenite::tungstenite::Message> =
        tokio::time::timeout(WAIT, wired.client.next())
            .await
            .ok()
            .flatten()
            .and_then(Result::ok);
    assert_eq!(
        received,
        Some(tokio_tungstenite::tungstenite::Message::binary(
            Bytes::from_static(b"keyframe")
        ))
    );
}

#[tokio::test]
async fn a_closing_phone_leaves_and_releases_its_buttons() {
    let mut wired: Wired = wired().await;
    say(&mut wired.client, r#"{"t":"visible","v":true}"#).await;
    desk_input(&mut wired.desk).await;
    wired.client.close(None).await.unwrap();
    assert_eq!(
        (
            desk_input(&mut wired.desk).await,
            injected(&mut wired.input).await
        ),
        (
            Some(DeskInput::ViewerLeft),
            Some(InputCommand::Button {
                button: Button::Left,
                pressed: false
            })
        )
    );
}
