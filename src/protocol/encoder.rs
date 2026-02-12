use crate::{
    QuestDevice,
    com::oculus::companion::server::{Method, Request},
};
use btleplug::api::{Peripheral, WriteType};
use log::*;
use prost::Message;
use std::error::Error;

pub struct PacketAssembler {
    buffer: Vec<u8>,
    next_seq: u16,
}

impl Default for PacketAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl PacketAssembler {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            next_seq: 0,
        }
    }

    pub fn handle_notification(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < 2 {
            return None;
        }

        let byte0 = data[0];
        let byte1 = data[1];

        let end_flag = (byte0 & 0x80) != 0;
        let seq_high = (byte0 & 0x1F) as u16;
        let seq_low = byte1 as u16;
        let seq = (seq_high << 8) | seq_low;

        let payload = &data[2..];

        if seq != self.next_seq {
            self.buffer.clear();
            self.next_seq = 0;

            if seq == 0 {
                self.buffer.extend_from_slice(payload);
                self.next_seq = 1;

                if end_flag {
                    let full_data = std::mem::take(&mut self.buffer);
                    self.next_seq = 0;
                    return Some(full_data);
                }
            }
            return None;
        }

        self.buffer.extend_from_slice(payload);
        self.next_seq += 1;

        if end_flag {
            let full_data = std::mem::take(&mut self.buffer);
            self.next_seq = 0;
            Some(full_data)
        } else {
            None
        }
    }
}

fn fragment_message(data: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let max_ble_data = mtu.saturating_sub(3);
    let payload_size = max_ble_data.saturating_sub(2);

    if payload_size == 0 {
        return Vec::new();
    }

    let mut packets = Vec::new();
    let mut offset = 0;
    let mut seq: u16 = 0;

    while offset < data.len() {
        let end = std::cmp::min(offset + payload_size, data.len());
        let chunk = &data[offset..end];
        offset += chunk.len();

        let is_last = offset >= data.len();

        let flags = if is_last { 0x80 } else { 0x00 };
        let seq_high = ((seq >> 8) & 0x1F) as u8;
        let seq_low = (seq & 0xFF) as u8;

        let mut packet = Vec::with_capacity(2 + chunk.len());
        packet.push(flags | seq_high);
        packet.push(seq_low);
        packet.extend_from_slice(chunk);

        packets.push(packet);
        seq = seq.wrapping_add(1);
    }

    packets
}

pub async fn send_protobuf<T: prost::Message>(
    protobuf: T,
    method: Method,
    quest: &QuestDevice,
) -> Result<(), Box<dyn Error>> {
    let mut proto_bytes = Vec::new();
    protobuf.encode(&mut proto_bytes)?;

    let req = Request {
        version: Some(1),
        method: Some(method.into()),
        seq: Some(0),
        body: Some(proto_bytes),
    };

    let mut req_bytes = Vec::new();
    req.encode(&mut req_bytes)?;

    let packets = fragment_message(&req_bytes, 23);

    debug!("Sending {:?}, {} packets", method, packets.len());

    for p in packets.iter() {
        quest
            .peripheral
            .write(&quest.ccs_characteristic, p, WriteType::WithResponse)
            .await?;
    }

    Ok(())
}

// completely llm generated tests - veygax
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fragment_message_simple() {
        let data = b"Hello World";
        let mtu = 20; // max_ble=17, payload=15. Large enough for single packet.
        let packets = fragment_message(data, mtu);

        assert_eq!(packets.len(), 1);
        let p = &packets[0];
        assert_eq!(p.len(), 2 + 11);
        // Header: End(0x80) | seq_high(0) = 0x80. seq_low(0) = 0x00.
        assert_eq!(p[0], 0x80);
        assert_eq!(p[1], 0x00);
        assert_eq!(&p[2..], data);
    }

    #[test]
    fn test_fragment_message_split() {
        let data = b"Hello World This Is A Long Message"; // 34 bytes
        // mtu = 10 -> max_ble = 7 -> payload = 5
        let mtu = 10;
        let packets = fragment_message(data, mtu);

        // 34 / 5 = 6 packets (5*6 = 30) -> 7 packets (last has 4)
        assert_eq!(packets.len(), 7);

        // Packet 0: seq 0, not end
        assert_eq!(packets[0][0], 0x00); // 0 | 0
        assert_eq!(packets[0][1], 0x00);
        assert_eq!(&packets[0][2..], b"Hello");

        // Packet 6: seq 6, end
        assert_eq!(packets[6][0], 0x80); // End
        assert_eq!(packets[6][1], 0x06);
        assert_eq!(&packets[6][2..], b"sage");
    }

    #[test]
    fn test_reassembly_simple() {
        let mut assembler = PacketAssembler::new();
        let data = b"TestPayload";
        let packet = vec![
            0x80, 0x00, b'T', b'e', b's', b't', b'P', b'a', b'y', b'l', b'o', b'a', b'd',
        ];

        let result = assembler.handle_notification(&packet);
        assert_eq!(result, Some(data.to_vec()));
    }

    #[test]
    fn test_reassembly_split() {
        let mut assembler = PacketAssembler::new();
        let expected = b"Hello World This Is A Long Message".to_vec();

        // Use fragment helper to generate packets
        let packets = fragment_message(&expected, 10);

        let mut result = None;
        for p in packets {
            result = assembler.handle_notification(&p);
        }

        assert_eq!(result, Some(expected));
    }

    #[test]
    fn test_sequence_mismatch_reset() {
        let mut assembler = PacketAssembler::new();

        // Send packet seq 0 (not end)
        let _ = assembler.handle_notification(&[0x00, 0x00, 0xAA]); // next_seq -> 1

        // Send packet seq 5 (unexpected)
        let result = assembler.handle_notification(&[0x80, 0x05, 0xBB]);

        // Should ignore/reset and return None
        assert_eq!(result, None);
        assert_eq!(assembler.next_seq, 0); // Reset
    }

    #[test]
    fn test_sequence_reset_with_start() {
        let mut assembler = PacketAssembler::new();

        // Send packet seq 0 (not end)
        assembler.handle_notification(&[0x00, 0x00, 0xAA]); // next_seq -> 1

        // Send packet seq 0 again (unexpected reset, new start)
        // And let's make it an END packet too (single packet message)
        let result = assembler.handle_notification(&[0x80, 0x00, 0xCC]);

        // Should process as new message
        assert_eq!(result, Some(vec![0xCC]));
    }
}
