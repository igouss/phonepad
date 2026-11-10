use super::*;

pub struct Sizes {
    pub state: u64,
    pub deltas: (u32, u32),
}

impl Sizes {
    pub fn next(&mut self, kind: UnitKind) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        match kind {
            UnitKind::Keyframe => KEYFRAME_BYTES,
            UnitKind::Delta => {
                let span: u64 = u64::from(self.deltas.1.saturating_sub(self.deltas.0).max(1));
                self.deltas.0 + u32::try_from((self.state >> 33) % span).unwrap_or(0)
            }
        }
    }
}

pub enum Downlink {
    WebSocket(
        futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>,
    ),
    WebTransport {
        connection: Connection,
        echo: wtransport::SendStream,
    },
}

impl Downlink {
    pub async fn unit(&mut self, seq: u32, message: Vec<u8>) -> Result<(), String> {
        match self {
            Downlink::WebSocket(sink) => sink
                .send(Message::binary(message))
                .await
                .map_err(|e| format!("ws send: {e}")),
            Downlink::WebTransport { connection, .. } => {
                let connection: Connection = connection.clone();
                tokio::spawn(async move {
                    let sent: Result<(), String> = async {
                        let mut stream: wtransport::SendStream = connection
                            .open_uni()
                            .await
                            .map_err(|e| e.to_string())?
                            .await
                            .map_err(|e| e.to_string())?;
                        stream.set_priority(i32::MAX - i32::try_from(seq).unwrap_or(i32::MAX));
                        stream
                            .write_all(&message)
                            .await
                            .map_err(|e| e.to_string())?;
                        stream.finish().await.map_err(|e| e.to_string())
                    }
                    .await;
                    if let Err(error) = sent {
                        eprintln!("wt unit {seq}: {error}");
                    }
                });
                Ok(())
            }
        }
    }

    pub async fn echo(&mut self, record: [u8; PING_BYTES]) -> Result<(), String> {
        match self {
            Downlink::WebSocket(sink) => {
                let mut message: Vec<u8> = vec![ECHO_TAG];
                message.extend_from_slice(&record);
                sink.send(Message::binary(message))
                    .await
                    .map_err(|e| format!("ws echo: {e}"))
            }
            Downlink::WebTransport { echo, .. } => echo
                .write_all(&record)
                .await
                .map_err(|e| format!("wt echo: {e}")),
        }
    }
}

pub struct Sender {
    pub stream: Stream,
    pub want_keyframe: bool,
    pub last_keyframe: Option<Instant>,
    pub next_id: u32,
    pub next_seq: u32,
    pub sizes: Sizes,
    pub sent_at: BTreeMap<u32, u64>,
    pub log: Log,
    pub epoch: Instant,
}

impl Sender {
    pub fn step(&mut self, input: StreamInput) -> Vec<StreamCommand> {
        let decision: Decision<StreamEvent, StreamCommand> = stream::decide(&self.stream, input);
        self.log.desyncs += decision
            .events
            .iter()
            .filter(|event: &&StreamEvent| matches!(event, StreamEvent::Desynced))
            .count();
        self.stream = decision.events.iter().fold(
            std::mem::replace(&mut self.stream, Stream::Hidden),
            stream::evolve,
        );
        if decision.commands.contains(&StreamCommand::RequestKeyframe) {
            self.want_keyframe = true;
        }
        decision.commands
    }

    pub async fn tick(&mut self, downlink: &mut Downlink) -> Result<(), String> {
        let spaced: bool = self
            .last_keyframe
            .is_none_or(|last: Instant| last.elapsed() >= KEYFRAME_SPACING);
        let kind: UnitKind = if self.want_keyframe && spaced {
            self.want_keyframe = false;
            self.last_keyframe = Some(Instant::now());
            UnitKind::Keyframe
        } else {
            UnitKind::Delta
        };
        self.next_id += 1;
        let id: u32 = self.next_id;
        let bytes: u32 = self.sizes.next(kind);
        let at_us: u64 = us(self.epoch);
        let commands: Vec<StreamCommand> = self.step(StreamInput::UnitReady(Offer {
            kind,
            unit: Sent { id, bytes, at_us },
        }));
        if commands.contains(&StreamCommand::SendUnit) {
            if kind == UnitKind::Keyframe {
                self.log.keyframes += 1;
            }
            let seq: u32 = self.next_seq;
            self.next_seq += 1;
            self.sent_at.insert(id, at_us);
            downlink
                .unit(seq, unit_message(seq, id, at_us, bytes))
                .await?;
        }
        Ok(())
    }

    pub fn ack(&mut self, id: u32) {
        let at_us: u64 = us(self.epoch);
        if let Some(sent) = self.sent_at.remove(&id) {
            self.log.acked.push((sent, at_us));
        }
        self.step(StreamInput::Ack { id, at_us });
    }
}

pub async fn send_units(
    deltas: (u32, u32),
    mut downlink: Downlink,
    mut uplink: mpsc::UnboundedReceiver<Up>,
    epoch: Instant,
    until: Instant,
) -> Result<Log, String> {
    let mut sender: Sender = Sender {
        stream: Stream::Hidden,
        want_keyframe: false,
        last_keyframe: None,
        next_id: 0,
        next_seq: 0,
        sizes: Sizes { state: 7, deltas },
        sent_at: BTreeMap::new(),
        log: Log::default(),
        epoch,
    };
    sender.step(StreamInput::Visibility(true));
    let mut ticks: tokio::time::Interval = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while Instant::now() < until {
        tokio::select! {
            _ = ticks.tick() => sender.tick(&mut downlink).await?,
            up = uplink.recv() => match up {
                Some(Up::Ack(id)) => sender.ack(id),
                Some(Up::Ping(record)) => downlink.echo(record).await?,
                None => break,
            },
        }
    }
    Ok(sender.log)
}
