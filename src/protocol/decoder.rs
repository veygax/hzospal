use crate::{QuestDevice, com::oculus::companion::server::Response};
use btleplug::api::Peripheral;
use log::*;
use prost::Message;
use std::error::Error;

struct PacketAssembler {
    buffer: Vec<u8>,
    next_seq: u16,
}

impl Default for PacketAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl PacketAssembler {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            next_seq: 0,
        }
    }

    fn handle_notification(&mut self, data: &[u8]) -> Option<Vec<u8>> {
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

pub async fn receive_protobuf<T: prost::Message + Default>(
    quest: &QuestDevice,
) -> Result<T, Box<dyn Error>> {
    let mut assembler = PacketAssembler::new();
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed().as_secs() > 30 {
            return Err("Timeout waiting for response".into());
        }

        let data = quest.peripheral.read(&quest.ccs_characteristic).await?;

        if data.is_empty() {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            continue;
        }

        // 0xFF means the Quest has no data to give at this time, so just wait
        if data.len() == 1 && data[0] == 0xFF {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            continue;
        }

        if let Some(full_message) = assembler.handle_notification(&data) {
            debug!("Reassembled message: {} bytes", full_message.len());

            let response = Response::decode(&*full_message)?;
            debug!("Response Code: {:?}", response.code);
            debug!("Response Seq: {:?}", response.seq);

            if let Some(body) = response.body {
                let msg = T::decode(&*body)?;
                return Ok(msg);
            } else {
                return Err("Response body is missing".into());
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
}
