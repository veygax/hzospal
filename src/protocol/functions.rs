use crate::{
    QuestDevice,
    com::oculus::companion::server::{HelloRequest, HelloResponse, Method, Response},
    protocol::encoder::{PacketAssembler, send_protobuf},
};
use btleplug::api::Peripheral;
use log::*;
use prost::Message;
use rand::Rng;
use std::error::Error;

pub async fn say_hello(qp: &QuestDevice) -> Result<(), Box<dyn Error>> {
    let mut client_challenge = vec![0u8; 16];
    rand::rng().fill_bytes(&mut client_challenge);

    let public_key_bytes = qp.x25519_keypair.1.as_bytes().to_vec();

    let hello_request = HelloRequest {
        client_public_key: Some(public_key_bytes),
        client_challenge: Some(client_challenge),
        app_id: Some("com.oculus.companion.server".to_string()),
        app_version: Some("1.0.0".to_string()),
        ..Default::default()
    };

    if !qp.peripheral.is_connected().await? {
        return Err("Device is not connected".into());
    }

    debug!("Sending HelloRequest...");
    send_protobuf(hello_request, Method::Hello, qp).await?;

    debug!("Waiting for HelloResponse...");
    let mut assembler = PacketAssembler::new();
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed().as_secs() > 30 {
            return Err("Timeout waiting for response".into());
        }

        let data = qp.peripheral.read(&qp.ccs_characteristic).await?;

        if data.is_empty() {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            continue;
        }

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
                let hello_resp = HelloResponse::decode(&*body)?;
                debug!("Decoded HelloResponse:");
                debug!("  Has Signature: {}", hello_resp.signature.is_some());
                debug!(
                    "  Cert Length: {}",
                    hello_resp
                        .server_certificate
                        .as_ref()
                        .map(|c| c.len())
                        .unwrap_or(0)
                );
            }

            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    Ok(())
}
