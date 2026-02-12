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

use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter,
};
use btleplug::platform::{Manager, Peripheral, PeripheralId};
use com::oculus::companion::server;
use futures::stream::StreamExt;
use std::error::Error;
use uuid::{Uuid, uuid};

#[derive(Debug)]
pub struct QuestPeripheral {
    pub peripheral: Peripheral,
    pub name: String,
    pub id: PeripheralId,
    pub rssi: i16,
    pub ccs_characteristic: Option<Characteristic>,
    pub status_characteristic: Option<Characteristic>,
}

pub async fn scan_for_quest(
    tx: tokio::sync::mpsc::UnboundedSender<QuestPeripheral>,
) -> Result<(), Box<dyn Error>> {
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

                    peripheral.connect().await?;
                    peripheral.discover_services().await?;
                    let characteristics = peripheral.characteristics();

                    let ccs_characteristic =
                        characteristics.iter().find(|c| c.uuid == CCS_UUID).cloned();
                    let status_characteristic = characteristics
                        .iter()
                        .find(|c| c.uuid == STATUS_UUID)
                        .cloned();

                    let _ = tx.send(QuestPeripheral {
                        peripheral,
                        name,
                        id,
                        rssi,
                        ccs_characteristic,
                        status_characteristic,
                    });
                }
            }
        }
    }

    Ok(())
}
