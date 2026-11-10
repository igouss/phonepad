use super::*;

pub async fn ws_server(
    listener: TcpListener,
    deltas: (u32, u32),
    epoch: Instant,
    until: Instant,
) -> Result<Log, String> {
    let (tcp, _): (TcpStream, SocketAddr) = listener
        .accept()
        .await
        .map_err(|e| format!("ws accept: {e}"))?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    let socket: tokio_tungstenite::WebSocketStream<TcpStream> =
        tokio_tungstenite::accept_async(tcp)
            .await
            .map_err(|e| format!("ws upgrade: {e}"))?;
    let (sink, mut source) = socket.split();
    let (up, uplink): (mpsc::UnboundedSender<Up>, mpsc::UnboundedReceiver<Up>) =
        mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(Ok(message)) = source.next().await {
            if let Message::Binary(bytes) = message
                && let Some(parsed) = ws_up(&bytes)
                && up.send(parsed).is_err()
            {
                break;
            }
        }
    });
    send_units(deltas, Downlink::WebSocket(sink), uplink, epoch, until).await
}

pub async fn ws_viewer(port: u16, epoch: Instant, until: Instant) -> Result<Viewer, String> {
    let tcp: TcpStream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|e| format!("ws connect: {e}"))?;
    tcp.set_nodelay(true).map_err(|e| e.to_string())?;
    let (socket, _) = tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}/"), tcp)
        .await
        .map_err(|e| format!("ws handshake: {e}"))?;
    let (mut sink, mut source) = socket.split();
    let (events, mut inbox): (
        mpsc::UnboundedSender<ViewerEvent>,
        mpsc::UnboundedReceiver<ViewerEvent>,
    ) = mpsc::unbounded_channel();
    let reading: mpsc::UnboundedSender<ViewerEvent> = events.clone();
    tokio::spawn(async move {
        while let Some(Ok(message)) = source.next().await {
            let event: Option<ViewerEvent> = match message {
                Message::Binary(bytes) => match bytes.first() {
                    Some(&UNIT_TAG) => parse_unit(&bytes).map(ViewerEvent::Unit),
                    Some(&ECHO_TAG) => bytes
                        .get(1..1 + PING_BYTES)
                        .and_then(|record: &[u8]| record.try_into().ok())
                        .map(ViewerEvent::Echo),
                    Some(_) | None => None,
                },
                Message::Text(_)
                | Message::Ping(_)
                | Message::Pong(_)
                | Message::Close(_)
                | Message::Frame(_) => None,
            };
            if let Some(event) = event
                && reading.send(event).is_err()
            {
                break;
            }
        }
    });
    tokio::spawn(pinger(events, epoch, until));
    let mut viewer: Viewer = Viewer::new(epoch);
    while Instant::now() < until {
        let event: Option<ViewerEvent> = match tokio::time::timeout(
            until.saturating_duration_since(Instant::now()),
            inbox.recv(),
        )
        .await
        {
            Ok(event) => event,
            Err(_) => break,
        };
        match event {
            Some(ViewerEvent::Unit(arrival)) => {
                for id in viewer.arrive(arrival) {
                    let mut ack: Vec<u8> = vec![ACK_TAG];
                    ack.extend_from_slice(&id.to_be_bytes());
                    sink.send(Message::binary(ack))
                        .await
                        .map_err(|e| format!("ws ack: {e}"))?;
                }
            }
            Some(ViewerEvent::Echo(record)) => viewer.echoed(&record),
            Some(ViewerEvent::Ping(seq, at_us)) => {
                viewer.pings.insert(seq, (at_us, None));
                let mut ping: Vec<u8> = vec![PING_TAG];
                ping.extend_from_slice(&ping_record(seq, at_us));
                sink.send(Message::binary(ping))
                    .await
                    .map_err(|e| format!("ws ping: {e}"))?;
            }
            None => break,
        }
    }
    Ok(viewer)
}
