use crate::{
    QuestDevice,
    com::oculus::companion::server::{HelloRequest, HelloResponse, Method},
    protocol::{decoder::receive_protobuf, encoder::send_protobuf},
};
use btleplug::api::Peripheral;
use log::*;
use rand::Rng;
use std::error::Error;

pub async fn say_hello(quest: &QuestDevice) -> Result<(), Box<dyn Error>> {
    let mut client_challenge = vec![0u8; 16];
    rand::rng().fill_bytes(&mut client_challenge);

    let public_key_bytes = quest.x25519_keypair.1.as_bytes().to_vec();

    let hello_request = HelloRequest {
        client_public_key: Some(public_key_bytes),
        client_challenge: Some(client_challenge),
        app_id: Some("com.oculus.companion.server".to_string()),
        app_version: Some("1.0.0".to_string()),
        ..Default::default()
    };

    if !quest.peripheral.is_connected().await? {
        return Err("Device is not connected".into());
    }

    debug!("Sending HelloRequest...");
    send_protobuf(quest, hello_request, Method::Hello).await?;

    debug!("Waiting for HelloResponse...");
    let hello_resp = receive_protobuf::<HelloResponse>(quest).await?;

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

    Ok(())
}
