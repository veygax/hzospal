pub mod protocol;

// absolutely disgusting package naming but I'm just following the docs for prost-build - veygax
pub mod com {
    pub mod oculus {
        pub mod companion {
            pub mod server {
                include!(concat!(env!("OUT_DIR"), "/com.oculus.companion.server.rs"));
            }
        }
    }
}

use crate::protocol::functions::{authenticate_device, claim_device, say_hello};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter,
};
use btleplug::platform::{Manager, Peripheral};
use crypto_box::aead::OsRng;
use crypto_box::{SalsaBox, SecretKey};
use futures::stream::StreamExt;
use log::*;
use std::error::Error;
use std::sync::atomic::AtomicI32;
use uuid::{Uuid, uuid};

pub struct QuestDevice {
    pub peripheral: Peripheral,
    pub name: String,
    pub ccs_characteristic: Characteristic,
    pub status_characteristic: Characteristic,
    pub x25519_keypair: (SecretKey, [u8; 32]),
    pub crypto_box: Option<SalsaBox>,
    pub sequence_number: AtomicI32,
    pub device_key: Option<[u8; 32]>,
}

pub async fn connect_to_quest(
    device_key: Option<[u8; 32]>,
) -> Result<Option<QuestDevice>, Box<dyn Error>> {
    const QUEST_UUID: Uuid = uuid!("0000feb8-0000-1000-8000-00805f9b34fb");
    const CCS_UUID: Uuid = uuid!("7a442881-509c-47fa-ac02-b06a37d9eb76");
    const STATUS_UUID: Uuid = uuid!("7a442666-509c-47fa-ac02-b06a37d9eb76");

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters.first().ok_or("No Bluetooth adapters discovered")?;

    let mut events = central.events().await?;

    central.start_scan(ScanFilter::default()).await?;

    while let Some(event) = events.next().await {
        let id = match event {
            CentralEvent::DeviceDiscovered(id) => id,
            CentralEvent::DeviceUpdated(id) => id,
            _ => continue,
        };

        if let Ok(peripheral) = central.peripheral(&id).await {
            if let Some(properties) = peripheral.properties().await? {
                if properties.services.contains(&QUEST_UUID) {
                    let name = properties
                        .local_name
                        .as_deref()
                        .unwrap_or("Unknown")
                        .to_string();

                    let rssi = properties.rssi.unwrap_or(0);

                    debug!("Found {}, {} RSSI", name, rssi);

                    central.stop_scan().await?;

                    debug!("Connecting to {}...", name);
                    peripheral.connect().await?;
                    debug!("Connected.");

                    peripheral.discover_services().await?;
                    let characteristics = peripheral.characteristics();

                    let ccs_characteristic = characteristics
                        .iter()
                        .find(|c| c.uuid == CCS_UUID)
                        .cloned()
                        .ok_or("Failed to find CCS characteristic")?;

                    let status_characteristic = characteristics
                        .iter()
                        .find(|c| c.uuid == STATUS_UUID)
                        .cloned()
                        .ok_or("Failed to find status characteristic")?;

                    let x25519_keypair: (SecretKey, [u8; 32]) = generate_x25519_keypair();

                    let mut quest = QuestDevice {
                        peripheral,
                        name,
                        ccs_characteristic,
                        status_characteristic,
                        x25519_keypair,
                        crypto_box: None,
                        sequence_number: AtomicI32::new(0),
                        device_key,
                    };

                    let challenge = say_hello(&mut quest).await?;

                    // the Quest only returns a challenge if it's claimed
                    match challenge {
                        Some(c) => authenticate_device(&quest, c).await?,
                        None => claim_device(&mut quest, device_key).await?,
                    };

                    debug!("Authenticated device!");

                    return Ok(Some(quest));
                }
            }
        }
    }

    Ok(None)
}

fn generate_x25519_keypair() -> (SecretKey, [u8; 32]) {
    let secret_key = SecretKey::generate(&mut OsRng);
    let public_key = secret_key.public_key().as_bytes().clone();
    (secret_key, public_key)
}
