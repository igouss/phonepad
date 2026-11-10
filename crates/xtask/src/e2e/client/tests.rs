use super::{Handshake, LOGIN_HEADER, Received, ack, handshake, request, timed_out, watch};
use std::io::{Error, ErrorKind};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tungstenite::handshake::client::Request;
use tungstenite::handshake::server::{Callback, ErrorResponse, Response};
use tungstenite::http::StatusCode;
use tungstenite::{Message, WebSocket};

const LOGIN: &str = "test@example.invalid";
const WATCHED: Duration = Duration::from_millis(600);

fn server<T: Send + 'static>(
    serve: impl FnOnce(TcpStream) -> T + Send + 'static,
) -> (u16, JoinHandle<T>) {
    let listener: TcpListener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port: u16 = listener.local_addr().unwrap().port();
    (
        port,
        thread::spawn(move || serve(listener.accept().unwrap().0)),
    )
}

struct LoginGate;

impl Callback for LoginGate {
    fn on_request(
        self,
        request: &tungstenite::handshake::server::Request,
        response: Response,
    ) -> Result<Response, ErrorResponse> {
        if request.headers()[LOGIN_HEADER] == LOGIN {
            Ok(response)
        } else {
            let mut refusal: ErrorResponse = ErrorResponse::new(None);
            *refusal.status_mut() = StatusCode::FORBIDDEN;
            Err(refusal)
        }
    }
}

fn accept(stream: TcpStream) -> WebSocket<TcpStream> {
    tungstenite::accept_hdr(stream, LoginGate).unwrap()
}

fn text(socket: &mut WebSocket<TcpStream>) -> String {
    socket
        .read()
        .unwrap()
        .into_text()
        .unwrap()
        .as_str()
        .to_owned()
}

fn upgraded(port: u16) -> Box<WebSocket<TcpStream>> {
    match handshake(port, Some(LOGIN)).unwrap() {
        Handshake::Upgraded(socket) => socket,
        Handshake::Refused(status) => panic!("refused with {status}"),
    }
}

const CURSOR: [u8; 9] = [1, 0, 0, 0, 5, 0, 0, 0, 6];

fn frame(id: u8) -> Vec<u8> {
    vec![0, 0, 0, 0, id, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0x65]
}

#[test]
fn a_frame_is_acked_by_its_id() {
    assert_eq!(
        ack(&[0, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        Some(String::from(r#"{"t":"ack","id":16909060}"#))
    );
}

#[test]
fn a_cursor_message_is_not_acked() {
    assert_eq!(ack(&CURSOR), None);
}

#[test]
fn a_frame_shorter_than_its_header_is_not_acked() {
    assert_eq!(ack(&[0, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0]), None);
}

#[test]
fn an_empty_message_is_not_acked() {
    assert_eq!(ack(&[]), None);
}

fn phone_session(stream: TcpStream) -> Vec<String> {
    let mut socket: WebSocket<TcpStream> = accept(stream);
    let opening: [String; 2] = [text(&mut socket), text(&mut socket)];
    socket.send(Message::binary(frame(7))).unwrap();
    socket.send(Message::text("hello")).unwrap();
    socket.send(Message::binary(CURSOR.to_vec())).unwrap();
    socket.send(Message::binary(frame(8))).unwrap();
    let acks: [String; 2] = [text(&mut socket), text(&mut socket)];
    let closing: Message = socket.read().unwrap();
    opening
        .into_iter()
        .chain(acks)
        .chain([format!("close: {}", closing.is_close())])
        .collect()
}

#[test]
fn a_viewer_opens_with_view_and_visible_and_acks_each_frame_by_its_id() {
    let (port, phone): (u16, JoinHandle<Vec<String>>) = server(phone_session);
    let received: Received = watch(upgraded(port), (648, 1072), WATCHED).unwrap();
    assert_eq!(
        (received.binary, received.texts, phone.join().unwrap()),
        (
            vec![frame(7), CURSOR.to_vec(), frame(8)],
            1,
            vec![
                String::from(r#"{"t":"view","w":648,"h":1072}"#),
                String::from(r#"{"t":"visible","v":true}"#),
                String::from(r#"{"t":"ack","id":7}"#),
                String::from(r#"{"t":"ack","id":8}"#),
                String::from("close: true"),
            ]
        )
    );
}

#[test]
fn a_dropped_connection_ends_the_watch_with_a_read_error() {
    let (port, phone): (u16, JoinHandle<()>) = server(|stream: TcpStream| {
        let mut socket: WebSocket<TcpStream> = accept(stream);
        let _opening: [String; 2] = [text(&mut socket), text(&mut socket)];
    });
    let ended: Result<Received, String> = watch(upgraded(port), (648, 1072), WATCHED);
    phone.join().unwrap();
    assert!(
        ended
            .err()
            .is_some_and(|error: String| error.starts_with("read:"))
    );
}

#[test]
fn a_wrong_login_is_refused_with_its_status() {
    let (port, phone): (u16, JoinHandle<bool>) =
        server(|stream: TcpStream| tungstenite::accept_hdr(stream, LoginGate).is_err());
    let refused: Option<u16> = match handshake(port, Some("intruder@example.invalid")).unwrap() {
        Handshake::Upgraded(_) => None,
        Handshake::Refused(status) => Some(status),
    };
    assert_eq!((refused, phone.join().unwrap()), (Some(403), true));
}

#[test]
fn a_read_timeout_is_a_poll_tick() {
    assert!(timed_out(&tungstenite::Error::Io(Error::from(
        ErrorKind::WouldBlock
    ))));
}

#[test]
fn a_timed_out_read_is_a_poll_tick() {
    assert!(timed_out(&tungstenite::Error::Io(Error::from(
        ErrorKind::TimedOut
    ))));
}

#[test]
fn a_reset_is_not_a_poll_tick() {
    assert!(!timed_out(&tungstenite::Error::Io(Error::from(
        ErrorKind::ConnectionReset
    ))));
}

#[test]
fn a_closed_connection_is_not_a_poll_tick() {
    assert!(!timed_out(&tungstenite::Error::ConnectionClosed));
}

#[test]
fn the_request_targets_the_socket_path_on_the_port() {
    assert_eq!(
        request(8769, None).map(|r: Request| r.uri().to_string()),
        Ok(String::from("ws://127.0.0.1:8769/ws"))
    );
}

#[test]
fn no_login_sends_no_login_header() {
    assert_eq!(
        request(8769, None).map(|r: Request| r.headers().contains_key(LOGIN_HEADER)),
        Ok(false)
    );
}

#[test]
fn a_login_is_sent_in_the_login_header() {
    assert_eq!(
        request(8769, Some(LOGIN))
            .map(|r: Request| r.headers()[LOGIN_HEADER].to_str().map(str::to_owned).ok()),
        Ok(Some(String::from(LOGIN)))
    );
}
