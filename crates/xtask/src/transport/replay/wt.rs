use super::*;

pub fn quic_transport(congestion: Congestion) -> wtransport::quinn::TransportConfig {
    let mut transport: wtransport::quinn::TransportConfig =
        wtransport::quinn::TransportConfig::default();
    match congestion {
        Congestion::Cubic => {}
        Congestion::Bbr => {
            transport.congestion_controller_factory(Arc::new(
                wtransport::quinn::congestion::BbrConfig::default(),
            ));
        }
    }
    transport
        .initial_mtu(QUIC_PAYLOAD)
        .min_mtu(QUIC_PAYLOAD)
        .mtu_discovery_config(None)
        .enable_segmentation_offload(false)
        .max_concurrent_uni_streams(wtransport::quinn::VarInt::from_u32(MAX_UNI_STREAMS))
        .max_idle_timeout(Some(
            wtransport::quinn::IdleTimeout::try_from(IDLE).unwrap_or(
                wtransport::quinn::IdleTimeout::from(wtransport::quinn::VarInt::from_u32(10_000)),
            ),
        ));
    transport
}

pub struct WtServer {
    pub endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    pub port: u16,
    pub hash: wtransport::tls::Sha256Digest,
}

pub fn wt_bind(port: u16, congestion: Congestion) -> Result<WtServer, String> {
    let identity: Identity =
        Identity::self_signed(["127.0.0.1", "localhost"]).map_err(|e| e.to_string())?;
    let hash: wtransport::tls::Sha256Digest = identity
        .certificate_chain()
        .as_slice()
        .first()
        .ok_or("the self-signed identity has no certificate")?
        .hash();
    let mut config: ServerConfig = ServerConfig::builder()
        .with_bind_address(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .with_identity(identity)
        .build();
    config
        .quic_config_mut()
        .transport_config(Arc::new(quic_transport(congestion)));
    let endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server> =
        Endpoint::server(config).map_err(|e| format!("wt bind: {e}"))?;
    let port: u16 = endpoint.local_addr().map_err(|e| e.to_string())?.port();
    Ok(WtServer {
        endpoint,
        port,
        hash,
    })
}

pub async fn read_role(stream: &mut wtransport::RecvStream) -> Result<u8, String> {
    let mut role: [u8; 1] = [0];
    stream
        .read_exact(&mut role)
        .await
        .map_err(|e| format!("wt role: {e}"))?;
    Ok(role[0])
}

pub async fn wt_server(
    server: WtServer,
    deltas: (u32, u32),
    epoch: Instant,
    until: Instant,
) -> Result<Log, String> {
    let request = server
        .endpoint
        .accept()
        .await
        .await
        .map_err(|e| format!("wt session: {e}"))?;
    let connection: Connection = request
        .accept()
        .await
        .map_err(|e| format!("wt accept: {e}"))?;
    let (up, uplink): (mpsc::UnboundedSender<Up>, mpsc::UnboundedReceiver<Up>) =
        mpsc::unbounded_channel();
    let mut echo: Option<wtransport::SendStream> = None;
    while echo.is_none() {
        let (send, mut recv): (wtransport::SendStream, wtransport::RecvStream) = connection
            .accept_bi()
            .await
            .map_err(|e| format!("wt accept_bi: {e}"))?;
        let up: mpsc::UnboundedSender<Up> = up.clone();
        match read_role(&mut recv).await? {
            CONTROL_ROLE => {
                drop(send);
                tokio::spawn(async move {
                    let mut record: [u8; 4] = [0; 4];
                    while recv.read_exact(&mut record).await.is_ok() {
                        if up.send(Up::Ack(u32::from_be_bytes(record))).is_err() {
                            break;
                        }
                    }
                });
            }
            PING_ROLE => {
                echo = Some(send);
                tokio::spawn(async move {
                    let mut record: [u8; PING_BYTES] = [0; PING_BYTES];
                    while recv.read_exact(&mut record).await.is_ok() {
                        if up.send(Up::Ping(record)).is_err() {
                            break;
                        }
                    }
                });
            }
            other => return Err(format!("wt stream with unknown role {other}")),
        }
    }
    let echo: wtransport::SendStream = echo.ok_or("no ping stream")?;
    let log: Log = send_units(
        deltas,
        Downlink::WebTransport {
            connection: connection.clone(),
            echo,
        },
        uplink,
        epoch,
        until,
    )
    .await?;
    connection.close(wtransport::VarInt::from_u32(0), b"done");
    Ok(log)
}

pub async fn wt_viewer(
    port: u16,
    hash: wtransport::tls::Sha256Digest,
    congestion: Congestion,
    epoch: Instant,
    until: Instant,
) -> Result<Viewer, String> {
    let mut config: ClientConfig = ClientConfig::builder()
        .with_bind_address(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .with_server_certificate_hashes([hash])
        .build();
    config
        .quic_config_mut()
        .transport_config(Arc::new(quic_transport(congestion)));
    let endpoint: Endpoint<wtransport::endpoint::endpoint_side::Client> =
        Endpoint::client(config).map_err(|e| format!("wt client: {e}"))?;
    let connection: Connection = endpoint
        .connect(format!("https://127.0.0.1:{port}/"))
        .await
        .map_err(|e| format!("wt connect: {e}"))?;
    let open_bi = |role: u8| {
        let connection: Connection = connection.clone();
        async move {
            let (mut send, recv): (wtransport::SendStream, wtransport::RecvStream) = connection
                .open_bi()
                .await
                .map_err(|e| format!("wt open_bi: {e}"))?
                .await
                .map_err(|e| format!("wt open_bi: {e}"))?;
            send.write_all(&[role])
                .await
                .map_err(|e| format!("wt role: {e}"))?;
            Ok::<(wtransport::SendStream, wtransport::RecvStream), String>((send, recv))
        }
    };
    let (mut control, _): (wtransport::SendStream, wtransport::RecvStream) =
        open_bi(CONTROL_ROLE).await?;
    let (mut pings_up, mut echoes): (wtransport::SendStream, wtransport::RecvStream) =
        open_bi(PING_ROLE).await?;
    let (events, mut inbox): (
        mpsc::UnboundedSender<ViewerEvent>,
        mpsc::UnboundedReceiver<ViewerEvent>,
    ) = mpsc::unbounded_channel();
    let units: mpsc::UnboundedSender<ViewerEvent> = events.clone();
    let accepting: Connection = connection.clone();
    tokio::spawn(async move {
        while let Ok(mut stream) = accepting.accept_uni().await {
            let units: mpsc::UnboundedSender<ViewerEvent> = units.clone();
            tokio::spawn(async move {
                let mut message: Vec<u8> = Vec::new();
                let mut buffer: [u8; 16_384] = [0; 16_384];
                while let Ok(Some(read)) = stream.read(&mut buffer).await {
                    message.extend_from_slice(&buffer[..read]);
                }
                if let Some(arrival) = parse_unit(&message) {
                    let _ = units.send(ViewerEvent::Unit(arrival));
                }
            });
        }
    });
    let echoing: mpsc::UnboundedSender<ViewerEvent> = events.clone();
    tokio::spawn(async move {
        let mut record: [u8; PING_BYTES] = [0; PING_BYTES];
        while echoes.read_exact(&mut record).await.is_ok() {
            if echoing.send(ViewerEvent::Echo(record)).is_err() {
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
                    control
                        .write_all(&id.to_be_bytes())
                        .await
                        .map_err(|e| format!("wt ack: {e}"))?;
                }
            }
            Some(ViewerEvent::Echo(record)) => viewer.echoed(&record),
            Some(ViewerEvent::Ping(seq, at_us)) => {
                viewer.pings.insert(seq, (at_us, None));
                pings_up
                    .write_all(&ping_record(seq, at_us))
                    .await
                    .map_err(|e| format!("wt ping: {e}"))?;
            }
            None => break,
        }
    }
    Ok(viewer)
}
