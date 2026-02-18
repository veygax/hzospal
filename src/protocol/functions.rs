use crate::{
    QuestDevice,
    com::oculus::companion::server::{
        AdbModeRequest, AdbModeResponse, AuthenticateRequest, CombinedSetAccessTokenRequest,
        DevModeRequest, DevModeResponse, HelloRequest, HelloResponse, HelloSignedData,
        HmdStatusResponse, ManagedAutoProvisioningStartRequest, Method, OculusSetUserSecretRequest,
        OtaEnabledRequest, OtaEnabledResponse, SkipNuxAndLoginRequest, SkipNuxAndLoginResponse,
        SkipNuxType,
    },
    protocol::{decoder::receive_protobuf, encoder::send_protobuf},
};
use crypto_box::{PublicKey, SalsaBox};
use hmac::{Hmac, Mac};
use log::*;
use prost::Message;
use rand::Rng;
use sha2::Sha256;
use std::error::Error;

type HmacSha256 = Hmac<Sha256>;

pub(crate) async fn say_hello(quest: &mut QuestDevice) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
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

    let data = hello_resp
        .signed_data
        .ok_or("HelloResponse was missing signed data")?;

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

    let auth_challenge = decoded.authentication_challenge;

    Ok(auth_challenge)
}

pub(crate) async fn claim_device(
    quest: &mut QuestDevice,
    device_key: Option<[u8; 32]>,
) -> Result<(), Box<dyn Error>> {
    let device_key = device_key.unwrap_or_else(|| {
        let mut key = [0u8; 32];
        rand::fill(&mut key);
        key
    });

    debug!("Claiming device (please do not disconnect the device)....");

    let claim_req = OculusSetUserSecretRequest {
        user_secret_key: Some(device_key.to_vec()),
    };

    send_protobuf(quest, Some(claim_req), Method::OculusSetUserSecret).await?;

    receive_protobuf::<()>(quest).await?;

    quest.device_key = Some(device_key);

    debug!(
        "Claimed under {} (hex-encoded) device key, please backup or else you may have to reset your device!",
        hex::encode(device_key)
    );

    Ok(())
}

pub(crate) async fn authenticate_device(
    quest: &QuestDevice,
    challenge: Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    let key = quest.device_key.ok_or("Device key not set")?;
    let mut hmac = HmacSha256::new_from_slice(&key)?;
    hmac.update(&challenge);
    let signed_challenge = hmac.finalize().into_bytes().to_vec();

    let auth_req = AuthenticateRequest {
        signed_authentication_challenge: Some(signed_challenge),
    };

    send_protobuf(quest, Some(auth_req), Method::Authenticate).await?;

    receive_protobuf::<()>(quest).await?;

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

pub async fn set_dev_mode(quest: &QuestDevice, mode: bool) -> Result<(), Box<dyn Error>> {
    let dev_req = DevModeRequest {
        mode: Some(mode.into()),
    };
    debug!("Asking to change dev mode to {}", mode);
    send_protobuf(quest, Some(dev_req), Method::DevModeSet).await?;

    // this does not say whether it was changed
    receive_protobuf::<()>(quest).await?;

    send_protobuf::<()>(quest, None, Method::DevModeStatus).await?;

    let dev_resp = receive_protobuf::<DevModeResponse>(quest).await?;

    debug!("Dev mode is now: {:#?}", dev_resp.status);

    Ok(())
}

pub async fn set_ota_mode(quest: &QuestDevice, mode: bool) -> Result<(), Box<dyn Error>> {
    let ota_req = OtaEnabledRequest { enable: Some(mode) };
    debug!("Asking to change OTA mode to {}", mode);
    send_protobuf(quest, Some(ota_req), Method::OtaEnabledSet).await?;

    // this does not say whether it was changed
    receive_protobuf::<()>(quest).await?;

    send_protobuf::<()>(quest, None, Method::OtaEnabledStatus).await?;

    let ota_resp = receive_protobuf::<OtaEnabledResponse>(quest).await?;

    debug!("OTA updates are now: {:#?}", ota_resp.enabled);

    Ok(())
}

pub async fn skip_nux(quest: &QuestDevice) -> Result<(), Box<dyn Error>> {
    let token_req = CombinedSetAccessTokenRequest {
        access_token_meta: Some("VEYGAX_HZOSPAL".into()),
        access_token_horizon_profile: Some("VEYGAX_HZOSPAL".into()),
        user_id: Some(1.to_string()),
        user_id_meta: Some(1.to_string()),
        ..Default::default()
    };

    send_protobuf(quest, Some(token_req), Method::MetaSetAccessTokenCombined).await?;

    debug!("Waiting for token to be set");
    let token_resp = receive_protobuf::<()>(quest).await?;

    debug!("Response to changing token: {:#?}", token_resp);

    debug!("Skipping NUX...");

    let nux_req = SkipNuxAndLoginRequest {
        skip_nux_type: Some(SkipNuxType::Default.into()),
        reboot: Some(false),
        do_not_disturb: Some(false),
        get_status: Some(false),
        disable_guardian: Some(false),
        ..Default::default()
    };

    send_protobuf(quest, Some(nux_req), Method::RetailSkipFirstTimeNux).await?;

    receive_protobuf::<()>(quest).await?;

    debug!("Waiting for NUX to finish skipping...");

    loop {
        let nux_req = SkipNuxAndLoginRequest {
            get_status: Some(true),
            ..Default::default()
        };

        send_protobuf(quest, Some(nux_req), Method::RetailSkipFirstTimeNux).await?;

        let resp = receive_protobuf::<SkipNuxAndLoginResponse>(quest).await?;
        if resp.status == Some(0) {
            break;
        }
    }

    debug!("NUX skipped!");

    Ok(())
}
