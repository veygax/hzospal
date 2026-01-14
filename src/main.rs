use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::Manager;
use futures::stream::StreamExt;
use std::error::Error;
use uuid::{Uuid, uuid};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    const QUEST_UUID: Uuid = uuid!("0000feb8-0000-1000-8000-00805f9b34fb");

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .nth(0)
        .ok_or("No Bluetooth adapters connected")?;

    let mut events = central.events().await?;

    println!("Scanning for Meta Quest devices...");
    central.start_scan(ScanFilter::default()).await?;

    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceDiscovered(id) => {
                let peripheral = central.peripheral(&id).await?;
                if let Some(properties) = peripheral.properties().await? {
                    if properties.services.contains(&QUEST_UUID) {
                        let name = properties
                            .local_name
                            .unwrap_or_else(|| "Unknown".to_string());
                        println!("Found Meta Quest: {:?}, Name: {}", id, name);
                    }
                }
            }
            _ => {
                // ignore other events for now
            }
        }
    }

    Ok(())
}
