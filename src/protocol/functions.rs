use crate::{
    QuestDevice,
    com::oculus::companion::server::{
        HelloRequest, HelloResponse, HelloSignedData, HmdStatusResponse, Method,
    },
    protocol::{decoder::receive_protobuf, encoder::send_protobuf},
};
use crypto_box::{PublicKey, SalsaBox};
use log::*;
use prost::Message;
use rand::Rng;
use std::error::Error;

pub(crate) async fn say_hello(quest: &mut QuestDevice) -> Result<(), Box<dyn Error>> {
    let mut client_challenge = vec![0u8; 16];
    rand::rng().fill_bytes(&mut client_challenge);

    let public_key_bytes = quest.x25519_keypair.1.to_vec();

    let hello_request = HelloRequest {
        client_public_key: Some(public_key_bytes),
        client_challenge: Some(client_challenge),
        app_id: Some("com.oculus.companion.server".to_string()),
        app_version: Some("1.0.0".to_string()),
        ..Default::default()
    };

    debug!("Sending HelloRequest...");
    send_protobuf(quest, Some(hello_request), Method::Hello).await?;

    debug!("Waiting for HelloResponse...");
    let hello_resp = receive_protobuf::<HelloResponse>(quest).await?;

    if let Some(data) = hello_resp.signed_data {
        let decoded = HelloSignedData::decode(&*data)?;
        debug!("Decoded signed HelloResponse: {:#?}", decoded);

        let server_public_key_bytes: [u8; 32] = decoded
            .server_public_key
            .ok_or("Failed to decode server public key")?
            .try_into()
            .map_err(|_| "Server public key is not 32 bytes")?;
        let server_public_key = PublicKey::from(server_public_key_bytes);

        quest.crypto_box = Some(SalsaBox::new(&server_public_key, &quest.x25519_keypair.0));

        debug!("Encryption setup");
    }

    Ok(())
}

pub async fn get_hmd_status(quest: &QuestDevice) -> Result<(), Box<dyn Error>> {
    debug!("Asking for status...");
    send_protobuf::<()>(quest, None, Method::HmdStatus).await?;

    debug!("Waiting for status...");
    let status_resp = receive_protobuf::<HmdStatusResponse>(quest).await?;

    debug!("Status: {:#?}", status_resp);

    Ok(())
}
