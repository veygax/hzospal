use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, PeripheralId};
use futures::stream::StreamExt;
use ratatui::{DefaultTerminal, Frame};
use std::error::Error;
use uuid::{Uuid, uuid};

struct QuestPeripheral {
    name: String,
    id: PeripheralId,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let quest_peripheral = scan_for_quest().await?;

    ratatui::run(|terminal| app(terminal, &quest_peripheral))?;

    Ok(())
}

async fn scan_for_quest() -> Result<QuestPeripheral, Box<dyn Error>> {
    const QUEST_UUID: Uuid = uuid!("0000feb8-0000-1000-8000-00805f9b34fb");

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters.first().ok_or("No Bluetooth adapters discovered")?;

    let mut events = central.events().await?;

    let mut quest_peripheral: Option<QuestPeripheral> = None;

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
                            .as_deref()
                            .unwrap_or("Unknown")
                            .to_string();

                        quest_peripheral = Some(QuestPeripheral { name: name, id: id });
                        central.stop_scan().await?;
                        break;
                    }
                }
            }
            _ => {
                // ignore other events for now
            }
        }
    }

    quest_peripheral.ok_or_else(|| "Failed to find a Meta Quest".into())
}

fn app(terminal: &mut DefaultTerminal, quest_peripheral: &QuestPeripheral) -> std::io::Result<()> {
    loop {
        terminal.draw(|frame| {
            render(frame, quest_peripheral);
        })?;
        if crossterm::event::read()?.is_key_press() {
            break Ok(());
        }
    }
}

fn render(frame: &mut Frame, quest_peripheral: &QuestPeripheral) {
    frame.render_widget(
        format!(
            "Found Meta Quest: {:?}, Name: {}",
            quest_peripheral.id, quest_peripheral.name
        ),
        frame.area(),
    );
}
