use crate::{
    QuestDevice,
    com::oculus::companion::server::{Method, Request},
};
use btleplug::api::{Peripheral, WriteType};
use crypto_box::{
    SalsaBox,
    aead::{Aead, AeadCore, OsRng},
};
use log::*;
use prost::Message;
use std::error::Error;

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

use std::sync::atomic::Ordering;

pub async fn send_protobuf<T: prost::Message>(
    quest: &QuestDevice,
    protobuf: Option<T>,
    method: Method,
) -> Result<(), Box<dyn Error>> {
    if !quest.peripheral.is_connected().await? {
        return Err("Device is not connected".into());
    }

    let body = if let Some(proto) = protobuf {
        let mut proto_bytes = Vec::new();
        proto.encode(&mut proto_bytes)?;

        Some(proto_bytes)
    } else {
        None
    };

    let seq = quest.sequence_number.fetch_add(1, Ordering::SeqCst);

    let req = Request {
        version: Some(1),
        method: Some(method.into()),
        seq: Some(seq),
        body,
        ..Default::default()
    };

    let mut req_bytes = Vec::new();
    req.encode(&mut req_bytes)?;

    // Hello is the only unencrypted method
    let data_to_send = if method != Method::Hello {
        let nonce = SalsaBox::generate_nonce(&mut OsRng);
        let mut ciphertext = quest
            .crypto_box
            .as_ref()
            .ok_or("No crypto_box")?
            .encrypt(&nonce, &req_bytes[..])
            .map_err(|_| "Encryption failed")?;

        let mut data = nonce.to_vec();
        data.append(&mut ciphertext);
        data
    } else {
        req_bytes
    };

    let packets = fragment_message(&data_to_send, 23);

    debug!("Sending {:?}, {} packets", method, packets.len());

    for p in packets.iter() {
        quest
            .peripheral
            .write(&quest.ccs_characteristic, p, WriteType::WithResponse)
            .await?;
    }

    Ok(())
}
