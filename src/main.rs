use hzospal::{
    QuestDevice, connect_to_quest,
    protocol::functions::{get_hmd_status, set_adb_mode},
};
//use ratatui::{DefaultTerminal, Frame};
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

    let quest: QuestDevice = connect_to_quest().await?.ok_or("Quest not found")?;

    get_hmd_status(&quest).await?;

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
