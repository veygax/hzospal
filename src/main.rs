use hzospal::{
    QuestDevice, connect_to_quest,
    protocol::functions::{get_hmd_status, set_dev_mode, set_ota_mode, skip_nux},
};
//use ratatui::{DefaultTerminal, Frame};
use directories::ProjectDirs;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<QuestPeripheral>();
    //
    // tokio::spawn(async move {
    //     let _ = scan_for_quest(tx).await;
    // });
    //
    // ratatui::run(|terminal| app(terminal, rx))?;

    env_logger::init();

    let proj_dirs = ProjectDirs::from("com", "veygax", "hzospal")
        .ok_or("Could not determine config directory")?;
    let config_dir = proj_dirs.config_dir();

    if !config_dir.exists() {
        std::fs::create_dir_all(config_dir)?;
    }

    let key_path = config_dir.join("device_key.bin");

    let device_key = if key_path.exists() {
        let key_vec = std::fs::read(&key_path)?;
        Some(
            key_vec
                .try_into()
                .map_err(|_| "device_key.bin > 32 bytes")?,
        )
    } else {
        None
    };

    let quest: QuestDevice = connect_to_quest(device_key)
        .await?
        .ok_or("Quest not found")?;

    if !key_path.exists() {
        if let Some(key) = quest.device_key {
            std::fs::write(&key_path, key)?;
            println!("Saved new device key to {:?}", key_path);
        }
    }

    get_hmd_status(&quest).await?;
    // skip_nux(&quest).await?;

    set_dev_mode(&quest, true).await?;
    set_ota_mode(&quest, false).await?;

    Ok(())
}

// remove ratatui until after protocol - veygax

// fn app(
//     terminal: &mut DefaultTerminal,
//     mut rx: tokio::sync::mpsc::UnboundedReceiver<QuestPeripheral>,
// ) -> Result<(), Box<dyn Error>> {
//     let mut quest_peripheral: Option<QuestPeripheral> = None;
//     loop {
//         while let Ok(qp) = rx.try_recv() {
//             quest_peripheral = Some(qp);
//         }
//
//         terminal.draw(|frame| {
//             render(frame, quest_peripheral.as_ref());
//         })?;
//         if crossterm::event::poll(std::time::Duration::from_millis(100))? {
//             if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
//                 if key.code == crossterm::event::KeyCode::Char('q') {
//                     break Ok(());
//                 }
//             }
//         }
//     }
// }
//
// fn render(frame: &mut Frame, quest_peripheral: Option<&QuestPeripheral>) {
//     let text = match quest_peripheral {
//         Some(qp) => {
//             let mut lines = vec![format!(
//                 "Found Meta Quest: {:?}, Name: {}, Rssi: {}",
//                 qp.id, qp.name, qp.rssi
//             )];
//
//             match (&qp.ccs_characteristic, &qp.status_characteristic) {
//                 (Some(ccs), Some(status)) => {
//                     lines.push("Found the two target characteristics!".to_string());
//                     lines.push(format!(
//                         "CCS UUID: {}, Properties: {:?}",
//                         ccs.uuid, ccs.properties
//                     ));
//                     lines.push(format!(
//                         "Status UUID: {}, Properties: {:?}",
//                         status.uuid, status.properties
//                     ));
//                 }
//                 _ => {
//                     lines.push(
//                         "Failed to find the two target characteristics. Check Bluetooth setup."
//                             .to_string(),
//                     );
//                 }
//             }
//
//             lines.join("\n")
//         }
//         None => "Scanning...".to_string(),
//     };
//
//     frame.render_widget(ratatui::widgets::Paragraph::new(text), frame.area());
// }
