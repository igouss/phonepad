use super::*;

pub fn unit_message(seq: u32, id: u32, sent_us: u64, bytes: u32) -> Vec<u8> {
    let mut message: Vec<u8> = Vec::with_capacity(bytes as usize);
    message.push(UNIT_TAG);
    message.extend_from_slice(&seq.to_be_bytes());
    message.extend_from_slice(&id.to_be_bytes());
    message.extend_from_slice(&sent_us.to_be_bytes());
    message.resize((bytes as usize).max(UNIT_HEADER), 0);
    message
}

pub fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub fn be64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

pub fn parse_unit(message: &[u8]) -> Option<Arrival> {
    Some(Arrival {
        seq: be32(message, 1)?,
        id: be32(message, 5)?,
        sent_us: be64(message, 9)?,
        bytes: message.len(),
    })
}

pub fn ping_record(seq: u32, at_us: u64) -> [u8; PING_BYTES] {
    let mut record: [u8; PING_BYTES] = [0; PING_BYTES];
    record[..4].copy_from_slice(&seq.to_be_bytes());
    record[4..].copy_from_slice(&at_us.to_be_bytes());
    record
}

pub enum Up {
    Ack(u32),
    Ping([u8; PING_BYTES]),
}

pub enum ViewerEvent {
    Unit(Arrival),
    Echo([u8; PING_BYTES]),
    Ping(u32, u64),
}

pub fn ws_up(message: &[u8]) -> Option<Up> {
    match *message.first()? {
        ACK_TAG => Some(Up::Ack(be32(message, 1)?)),
        PING_TAG => Some(Up::Ping(message.get(1..1 + PING_BYTES)?.try_into().ok()?)),
        _ => None,
    }
}
