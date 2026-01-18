use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, PeripheralId};
use futures::stream::StreamExt;
use ratatui::{DefaultTerminal, Frame};
use std::error::Error;
use uuid::{Uuid, uuid};

struct QuestPeripheral {
    name: String,
    id: PeripheralId,
    rssi: i16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<QuestPeripheral>();

    tokio::spawn(async move {
        let _ = scan_for_quest(tx).await;
    });

    ratatui::run(|terminal| app(terminal, rx))?;

    Ok(())
}

async fn scan_for_quest(
    tx: tokio::sync::mpsc::UnboundedSender<QuestPeripheral>,
) -> Result<(), Box<dyn Error>> {
    const QUEST_UUID: Uuid = uuid!("0000feb8-0000-1000-8000-00805f9b34fb");

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

                    let _ = tx.send(QuestPeripheral { name, id, rssi });
                }
            }
        }
    }

    Ok(())
}

fn app(
    terminal: &mut DefaultTerminal,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<QuestPeripheral>,
) -> Result<(), Box<dyn Error>> {
    let mut quest_peripheral: Option<QuestPeripheral> = None;
    loop {
        while let Ok(qp) = rx.try_recv() {
            quest_peripheral = Some(qp);
        }

        terminal.draw(|frame| {
            render(frame, quest_peripheral.as_ref());
        })?;
        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.code == crossterm::event::KeyCode::Char('q') {
                    break Ok(());
                }
            }
        }
    }
}

fn render(frame: &mut Frame, quest_peripheral: Option<&QuestPeripheral>) {
    let text = match quest_peripheral {
        Some(qp) => &format!(
            "Found Meta Quest: {:?}, Name: {}, Rssi: {}",
            qp.id, qp.name, qp.rssi
        ),
        None => "Scanning...",
    };

    frame.render_widget(text, frame.area());
}
