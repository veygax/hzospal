use directories::ProjectDirs;
use eframe::egui;
use hzospal::{
    QuestDevice, QuestPeripheral, connect_quest_peripheral,
    protocol::functions::{
        HmdStatusResponse, get_controller_status, get_dev_mode_status, get_hmd_status,
        get_ota_mode_status, set_dev_mode, set_ota_mode, skip_nux,
    },
    scan_quest_peripherals,
};
use log::{error, info, warn};
use std::{
    error::Error,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct DeviceSummary {
    index: usize,
    id: String,
    name: String,
    rssi: i16,
}

enum Screen {
    DeviceList,
    Controls,
}

enum QuestCommand {
    Scan,
    Connect(usize),
    GetStatus { silent: bool },
    SetDevMode(bool),
    SetOtaMode(bool),
    SkipNux,
}

enum QuestEvent {
    ScanStarted,
    Devices(Vec<DeviceSummary>),
    Connecting(DeviceSummary),
    Connected {
        name: String,
        saved_new_key: bool,
        key_path: PathBuf,
    },
    Status {
        status: ControlStatus,
        silent: bool,
    },
    DevModeSet(bool, String),
    OtaModeSet(bool, String),
    Completed(String),
    SilentStatusRefreshFailed(String),
    Failed(String),
}

struct ConnectedQuest {
    device: QuestDevice,
    saved_new_key: bool,
    key_path: PathBuf,
}

struct ControlStatus {
    hmd: HmdStatusResponse,
    dev_mode: Option<bool>,
    ota_mode: Option<bool>,
}

struct HzosPalApp {
    command_tx: Sender<QuestCommand>,
    event_rx: Receiver<QuestEvent>,
    screen: Screen,
    devices: Vec<DeviceSummary>,
    selected_device: Option<DeviceSummary>,
    connected_name: Option<String>,
    hmd_status: Option<HmdStatusResponse>,
    dev_mode: Option<bool>,
    ota_mode: Option<bool>,
    busy: bool,
    status_refresh_pending: bool,
    message: String,
    last_status_refresh: Instant,
}

impl HzosPalApp {
    fn new(
        command_tx: Sender<QuestCommand>,
        event_rx: Receiver<QuestEvent>,
        _cc: &eframe::CreationContext<'_>,
    ) -> Self {
        let app = Self {
            command_tx: command_tx.clone(),
            event_rx,
            screen: Screen::DeviceList,
            devices: Vec::new(),
            selected_device: None,
            connected_name: None,
            hmd_status: None,
            dev_mode: None,
            ota_mode: None,
            busy: true,
            status_refresh_pending: false,
            message: "Scanning for Meta Quest devices...".to_string(),
            last_status_refresh: Instant::now(),
        };

        if let Err(err) = command_tx.send(QuestCommand::Scan) {
            error!("failed to queue initial scan: {err}");
        }

        app
    }

    fn apply_event(&mut self, event: QuestEvent) {
        match event {
            QuestEvent::ScanStarted => {
                self.screen = Screen::DeviceList;
                self.busy = true;
                self.status_refresh_pending = false;
                self.message = "Scanning for Meta Quest devices...".to_string();
                info!("scanning for Quest devices");
            }
            QuestEvent::Devices(devices) => {
                self.busy = false;
                self.devices = devices;
                self.message = match self.devices.len() {
                    0 => "No Quest devices found. Make sure Bluetooth is on and the headset is nearby."
                        .to_string(),
                    1 => "1 Quest device found.".to_string(),
                    count => format!("{count} Quest devices found."),
                };
                info!("{}", self.message);
            }
            QuestEvent::Connecting(device) => {
                self.busy = true;
                self.selected_device = Some(device.clone());
                self.message = format!("Connecting to {}...", device.name);
                info!("{}", self.message);
            }
            QuestEvent::Connected {
                name,
                saved_new_key,
                key_path,
            } => {
                self.busy = true;
                self.screen = Screen::Controls;
                self.connected_name = Some(name.clone());
                self.hmd_status = None;
                self.dev_mode = None;
                self.ota_mode = None;
                self.status_refresh_pending = false;
                self.last_status_refresh = Instant::now();
                self.message = format!("Connected to {name}. Loading status...");
                info!("{}", self.message);
                if saved_new_key {
                    info!("saved new device key to {}", key_path.display());
                }
            }
            QuestEvent::Status { status, silent } => {
                self.dev_mode = status.dev_mode.or(status.hmd.developer_mode);
                self.ota_mode = status.ota_mode;
                self.hmd_status = Some(status.hmd);
                self.status_refresh_pending = false;
                self.busy = false;
                self.last_status_refresh = Instant::now();
                if !silent {
                    self.message = "Status refreshed.".to_string();
                }
                info!("status refreshed (silent: {silent})");
            }
            QuestEvent::DevModeSet(enabled, message) => {
                self.busy = false;
                self.dev_mode = Some(enabled);
                if let Some(status) = self.hmd_status.as_mut() {
                    status.developer_mode = Some(enabled);
                }
                self.message = message;
                info!("{}", self.message);
            }
            QuestEvent::OtaModeSet(enabled, message) => {
                self.busy = false;
                self.ota_mode = Some(enabled);
                self.message = message;
                info!("{}", self.message);
            }
            QuestEvent::Completed(message) => {
                self.busy = false;
                self.message = message;
                info!("{}", self.message);
            }
            QuestEvent::SilentStatusRefreshFailed(message) => {
                self.status_refresh_pending = false;
                self.last_status_refresh = Instant::now();
                warn!("{message}");
            }
            QuestEvent::Failed(message) => {
                self.busy = false;
                self.status_refresh_pending = false;
                self.message = message;
                error!("{}", self.message);
            }
        }
    }

    fn process_events(&mut self) {
        loop {
            match self.event_rx.try_recv() {
                Ok(event) => self.apply_event(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("bluetooth worker stopped");
                    self.busy = false;
                    self.message = "Bluetooth worker stopped.".to_string();
                    break;
                }
            }
        }
    }

    fn send_command(&mut self, command: QuestCommand, message: impl Into<String>) {
        if self.busy {
            return;
        }

        self.busy = true;
        self.message = message.into();

        if let Err(err) = self.command_tx.send(command) {
            self.busy = false;
            self.message = format!("Failed to queue command: {err}");
            error!("{}", self.message);
        }
    }

    fn show_devices(&mut self) {
        self.screen = Screen::DeviceList;
        self.hmd_status = None;
        self.send_command(QuestCommand::Scan, "Scanning for Meta Quest devices...");
    }

    fn maybe_refresh_status(&mut self) {
        if !self.busy
            && !self.status_refresh_pending
            && matches!(self.screen, Screen::Controls)
            && self.last_status_refresh.elapsed() >= Duration::from_secs(3)
        {
            self.last_status_refresh = Instant::now();
            self.status_refresh_pending = true;
            if let Err(err) = self
                .command_tx
                .send(QuestCommand::GetStatus { silent: true })
            {
                self.status_refresh_pending = false;
                error!("failed to send silent status refresh: {err}");
            }
        }
    }

    fn render_titlebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("hzospal");
            ui.separator();

            if self.busy {
                ui.label("Processing...");
            } else if let Some(name) = self.connected_name.as_ref() {
                ui.label(format!("Connected: {name}"));
            } else {
                ui.label("Ready");
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if matches!(self.screen, Screen::DeviceList)
                    && command_button(ui, "Refresh List", !self.busy)
                {
                    self.send_command(QuestCommand::Scan, "Scanning for Meta Quest devices...");
                }
            });
        });
    }

    fn render_device_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Quest Devices");
        ui.label("Select a Meta Quest headset to connect and control.");
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.devices.is_empty() {
                    empty_state(ui, self.busy);
                    return;
                }

                for device in self.devices.clone() {
                    self.device_row(ui, device);
                }
            });
    }

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        let connected_name = self
            .connected_name
            .as_deref()
            .unwrap_or("Quest")
            .to_string();
        let dev_mode = self.dev_mode.or_else(|| {
            self.hmd_status
                .as_ref()
                .and_then(|status| status.developer_mode)
        });
        let ota_mode = self.ota_mode;
        let status = self.hmd_status.clone();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(connected_name);
                ui.label(&self.message);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if command_button(ui, "Disconnect", !self.busy) {
                    self.show_devices();
                }
            });
        });
        ui.separator();

        if ui.available_width() > 620.0 {
            ui.columns(2, |columns| {
                self.setting_card(
                    &mut columns[0],
                    "Developer Mode",
                    "Allows developer features and ADB workflows on the headset.",
                    status_text(dev_mode, "Active", "Inactive", "Loading..."),
                    |app, ui| {
                        if toggle_button(ui, dev_mode, !app.busy) {
                            let enabled = !dev_mode.unwrap_or(false);
                            app.send_command(
                                QuestCommand::SetDevMode(enabled),
                                if enabled {
                                    "Enabling developer mode..."
                                } else {
                                    "Disabling developer mode..."
                                },
                            );
                        }
                    },
                );
                self.setting_card(
                    &mut columns[1],
                    "OTA Updates",
                    "Controls whether the headset accepts over-the-air updates.",
                    status_text(ota_mode, "Enabled", "Disabled", "Loading..."),
                    |app, ui| {
                        if toggle_button(ui, ota_mode, !app.busy) {
                            let enabled = !ota_mode.unwrap_or(false);
                            app.send_command(
                                QuestCommand::SetOtaMode(enabled),
                                if enabled {
                                    "Enabling OTA updates..."
                                } else {
                                    "Disabling OTA updates..."
                                },
                            );
                        }
                    },
                );
            });
            ui.add_space(16.0);
            ui.columns(2, |columns| {
                self.setting_card(
                    &mut columns[0],
                    "Skip NUX Setup",
                    "Bypasses the first-run new user experience prompts.",
                    "Action",
                    |app, ui| {
                        if command_button(ui, "Skip", !app.busy) {
                            app.send_command(QuestCommand::SkipNux, "Skipping NUX...");
                        }
                    },
                );
                self.setting_card(
                    &mut columns[1],
                    "Device Status",
                    "Refreshes hardware, charging, and connection details.",
                    if status.is_some() {
                        "Loaded"
                    } else {
                        "Not Loaded"
                    },
                    |app, ui| {
                        if command_button(ui, "Refresh", !app.busy) {
                            app.send_command(
                                QuestCommand::GetStatus { silent: false },
                                "Refreshing status...",
                            );
                        }
                    },
                );
            });
        } else {
            self.setting_card(
                ui,
                "Developer Mode",
                "Allows developer features and ADB workflows on the headset.",
                status_text(dev_mode, "Active", "Inactive", "Loading..."),
                |app, ui| {
                    if toggle_button(ui, dev_mode, !app.busy) {
                        let enabled = !dev_mode.unwrap_or(false);
                        app.send_command(
                            QuestCommand::SetDevMode(enabled),
                            if enabled {
                                "Enabling developer mode..."
                            } else {
                                "Disabling developer mode..."
                            },
                        );
                    }
                },
            );
            ui.add_space(12.0);
            self.setting_card(
                ui,
                "OTA Updates",
                "Controls whether the headset accepts over-the-air updates.",
                status_text(ota_mode, "Enabled", "Disabled", "Loading..."),
                |app, ui| {
                    if toggle_button(ui, ota_mode, !app.busy) {
                        let enabled = !ota_mode.unwrap_or(false);
                        app.send_command(
                            QuestCommand::SetOtaMode(enabled),
                            if enabled {
                                "Enabling OTA updates..."
                            } else {
                                "Disabling OTA updates..."
                            },
                        );
                    }
                },
            );
            ui.add_space(12.0);
            self.setting_card(
                ui,
                "Skip NUX Setup",
                "Bypasses the first-run new user experience prompts.",
                "Action",
                |app, ui| {
                    if command_button(ui, "Skip", !app.busy) {
                        app.send_command(QuestCommand::SkipNux, "Skipping NUX...");
                    }
                },
            );
            ui.add_space(12.0);
            self.setting_card(
                ui,
                "Device Status",
                "Refreshes hardware, charging, and connection details.",
                if status.is_some() {
                    "Loaded"
                } else {
                    "Not Loaded"
                },
                |app, ui| {
                    if command_button(ui, "Refresh", !app.busy) {
                        app.send_command(
                            QuestCommand::GetStatus { silent: false },
                            "Refreshing status...",
                        );
                    }
                },
            );
        }

        if let Some(status) = status.as_ref() {
            ui.add_space(16.0);
            status_content(ui, status);
        }
    }

    fn device_row(&mut self, ui: &mut egui::Ui, device: DeviceSummary) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.strong(device.name.clone());
                    ui.label(format!("{} dBm | {}", device.rssi, device.id));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if command_button(ui, "Connect", !self.busy) {
                        self.send_command(
                            QuestCommand::Connect(device.index),
                            format!("Connecting to {}...", device.name),
                        );
                    }
                });
            });
        });
    }

    fn setting_card(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        description: &str,
        status: &str,
        add_control: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.strong(title);
                ui.label(description);
                ui.horizontal(|ui| {
                    ui.label(format!("Status: {status}"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        add_control(self, ui);
                    });
                });
            });
        });
    }
}

impl eframe::App for HzosPalApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        self.maybe_refresh_status();
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("titlebar").show_inside(ui, |ui| {
            self.render_titlebar(ui);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| match self.screen {
            Screen::DeviceList => self.render_device_list(ui),
            Screen::Controls => self.render_controls(ui),
        });
    }
}

fn status_text(
    value: Option<bool>,
    active: &'static str,
    inactive: &'static str,
    loading: &'static str,
) -> &'static str {
    match value {
        Some(true) => active,
        Some(false) => inactive,
        None => loading,
    }
}

fn command_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}

fn toggle_button(ui: &mut egui::Ui, value: Option<bool>, enabled: bool) -> bool {
    let mut checked = value.unwrap_or(false);
    ui.add_enabled(
        enabled && value.is_some(),
        egui::Checkbox::new(&mut checked, "Enabled"),
    )
    .changed()
}

fn empty_state(ui: &mut egui::Ui, scanning: bool) {
    ui.group(|ui| {
        ui.label(if scanning {
            "Scanning for Quest devices..."
        } else {
            "No Quest devices found."
        });
        ui.label("Make sure Bluetooth is enabled and the headset is nearby.");
    });
}

fn status_content(ui: &mut egui::Ui, status: &HmdStatusResponse) {
    let battery_val = match status.battery_level {
        Some(level) => format!("{level}%"),
        None => "Unknown".to_string(),
    };

    let charging_val = match status.charging {
        Some(true) => "Charging".to_string(),
        Some(false) => "Not Charging".to_string(),
        None => "Unknown".to_string(),
    };

    let wifi_val = match status.wifi_connected {
        Some(true) => status
            .wifi_ip_address
            .as_deref()
            .unwrap_or("Connected")
            .to_string(),
        Some(false) => "Disconnected".to_string(),
        None => "Unknown".to_string(),
    };

    let adb_val = match status.adb_enabled {
        Some(true) => "Enabled".to_string(),
        Some(false) => "Disabled".to_string(),
        None => "Unknown".to_string(),
    };

    let primary_controller_val = controller_status_label(
        status.controller_primary_connected,
        status.controller_primary_battery_level,
    );
    let secondary_controller_val = controller_status_label(
        status.controller_secondary_connected,
        status.controller_secondary_battery_level,
    );

    ui.separator();
    ui.heading("Device Status");
    egui::Grid::new("status_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            status_row(ui, "Battery", &battery_val);
            status_row(ui, "Charging", &charging_val);
            status_row(ui, "Wi-Fi", &wifi_val);
            status_row(ui, "ADB", &adb_val);
            status_row(ui, "Primary Controller", &primary_controller_val);
            status_row(ui, "Secondary Controller", &secondary_controller_val);
        });
}

fn status_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.label(value);
    ui.end_row();
}

fn controller_status_label(connected: Option<bool>, battery_level: Option<i32>) -> String {
    match (connected, battery_level) {
        (Some(true), Some(level)) => format!("Connected ({level}%)"),
        (Some(true), None) => "Connected".to_string(),
        (Some(false), _) => "Disconnected".to_string(),
        (None, _) => "Unknown".to_string(),
    }
}

fn spawn_quest_worker(command_rx: Receiver<QuestCommand>, event_tx: Sender<QuestEvent>) {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                error!("failed to start Tokio runtime: {err}");
                let _ = event_tx.send(QuestEvent::Failed(format!(
                    "failed to start Tokio runtime: {err}"
                )));
                return;
            }
        };

        let mut quest: Option<QuestDevice> = None;
        let mut discovered: Vec<QuestPeripheral> = Vec::new();

        while let Ok(command) = command_rx.recv() {
            match command {
                QuestCommand::Scan => {
                    let _ = event_tx.send(QuestEvent::ScanStarted);
                    match runtime.block_on(scan_quest_peripherals(Duration::from_secs(5))) {
                        Ok(peripherals) => {
                            let summaries = peripherals
                                .iter()
                                .enumerate()
                                .map(|(index, peripheral)| DeviceSummary {
                                    index,
                                    id: peripheral.id.clone(),
                                    name: peripheral.name.clone(),
                                    rssi: peripheral.rssi,
                                })
                                .collect::<Vec<_>>();

                            discovered = peripherals;
                            let _ = event_tx.send(QuestEvent::Devices(summaries));
                        }
                        Err(err) => {
                            discovered.clear();
                            let _ =
                                event_tx.send(QuestEvent::Failed(format!("scan failed: {err}")));
                        }
                    }
                }
                QuestCommand::Connect(index) => {
                    if index >= discovered.len() {
                        let message =
                            "Selected Quest is no longer available. Refresh and try again.";
                        let _ = event_tx.send(QuestEvent::Failed(message.to_string()));
                        continue;
                    }

                    let summary = {
                        let peripheral = &discovered[index];
                        DeviceSummary {
                            index,
                            id: peripheral.id.clone(),
                            name: peripheral.name.clone(),
                            rssi: peripheral.rssi,
                        }
                    };
                    let peripheral = discovered.swap_remove(index);

                    let _ = event_tx.send(QuestEvent::Connecting(summary));
                    match runtime.block_on(connect_with_saved_key(peripheral)) {
                        Ok(connected) => {
                            let name = connected.device.name.clone();
                            let saved_new_key = connected.saved_new_key;
                            let key_path = connected.key_path;
                            quest = Some(connected.device);
                            let _ = event_tx.send(QuestEvent::Connected {
                                name,
                                saved_new_key,
                                key_path,
                            });

                            if let Some(quest) = quest.as_ref() {
                                match runtime.block_on(load_control_status(quest)) {
                                    Ok(status) => {
                                        let _ = event_tx.send(QuestEvent::Status {
                                            status,
                                            silent: false,
                                        });
                                    }
                                    Err(err) => {
                                        let _ = event_tx.send(QuestEvent::Failed(format!(
                                            "status refresh failed: {err}"
                                        )));
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            quest = None;
                            let _ =
                                event_tx.send(QuestEvent::Failed(format!("connect failed: {err}")));
                        }
                    }
                }
                QuestCommand::GetStatus { silent } => {
                    let result = runtime.block_on(async {
                        let quest = quest.as_ref().ok_or("Quest is not connected")?;
                        load_control_status(quest)
                            .await
                            .map_err(|err| err.to_string())
                    });
                    match result {
                        Ok(status) => {
                            let _ = event_tx.send(QuestEvent::Status { status, silent });
                        }
                        Err(err) => {
                            if !silent {
                                let _ = event_tx.send(QuestEvent::Failed(format!(
                                    "status refresh failed: {err}"
                                )));
                            } else {
                                let _ = event_tx.send(QuestEvent::SilentStatusRefreshFailed(
                                    format!("silent status refresh failed: {err}"),
                                ));
                            }
                        }
                    }
                }
                QuestCommand::SetDevMode(enabled) => {
                    let result = runtime.block_on(async {
                        let quest = quest.as_ref().ok_or("Quest is not connected")?;
                        set_dev_mode(quest, enabled)
                            .await
                            .map(|status| status.unwrap_or(enabled))
                            .map_err(|err| err.to_string())
                    });
                    match result {
                        Ok(status) => {
                            let message = format!("Developer mode set to {status}.");
                            let _ = event_tx.send(QuestEvent::DevModeSet(status, message));
                        }
                        Err(err) => {
                            let _ = event_tx.send(QuestEvent::Failed(format!(
                                "developer mode update failed: {err}"
                            )));
                        }
                    }
                }
                QuestCommand::SetOtaMode(enabled) => {
                    let result = runtime.block_on(async {
                        let quest = quest.as_ref().ok_or("Quest is not connected")?;
                        set_ota_mode(quest, enabled)
                            .await
                            .map(|status| status.unwrap_or(enabled))
                            .map_err(|err| err.to_string())
                    });
                    match result {
                        Ok(status) => {
                            let message = format!("OTA updates set to {status}.");
                            let _ = event_tx.send(QuestEvent::OtaModeSet(status, message));
                        }
                        Err(err) => {
                            let _ = event_tx
                                .send(QuestEvent::Failed(format!("OTA update failed: {err}")));
                        }
                    }
                }
                QuestCommand::SkipNux => {
                    let result = runtime.block_on(async {
                        let quest = quest.as_ref().ok_or("Quest is not connected")?;
                        skip_nux(quest).await.map_err(|err| err.to_string())
                    });
                    match result {
                        Ok(()) => {
                            let message = "NUX skipped.".to_string();
                            let _ = event_tx.send(QuestEvent::Completed(message));
                        }
                        Err(err) => {
                            let _ = event_tx
                                .send(QuestEvent::Failed(format!("skip NUX failed: {err}")));
                        }
                    }
                }
            }
        }
    });
}

async fn load_control_status(quest: &QuestDevice) -> Result<ControlStatus, Box<dyn Error>> {
    let mut primary_connected = None;
    let mut primary_battery = None;
    let mut secondary_connected = None;
    let mut secondary_battery = None;

    match get_controller_status(quest).await {
        Ok(resp) => {
            for ctrl in resp.paired_controllers {
                let is_connected = ctrl.state.map(|state| state != 0).unwrap_or(false);
                let battery = ctrl.battery_level;
                let ctrl_type = ctrl.r#type;

                if let Some(t) = ctrl_type {
                    if t == 0 {
                        primary_connected = Some(is_connected);
                        primary_battery = battery;
                    } else if t == 1 {
                        secondary_connected = Some(is_connected);
                        secondary_battery = battery;
                    }
                }
            }
        }
        Err(err) => {
            warn!("failed to query controller status: {err}");
        }
    }

    let mut hmd = get_hmd_status(quest).await?;

    if let Some(conn) = primary_connected {
        hmd.controller_primary_connected = Some(conn);
    }
    if let Some(batt) = primary_battery {
        hmd.controller_primary_battery_level = Some(batt);
    }
    if let Some(conn) = secondary_connected {
        hmd.controller_secondary_connected = Some(conn);
    }
    if let Some(batt) = secondary_battery {
        hmd.controller_secondary_battery_level = Some(batt);
    }

    let dev_mode = get_dev_mode_status(quest).await?;
    let ota_mode = get_ota_mode_status(quest).await?;

    Ok(ControlStatus {
        hmd,
        dev_mode,
        ota_mode,
    })
}

async fn connect_with_saved_key(
    quest_peripheral: QuestPeripheral,
) -> Result<ConnectedQuest, Box<dyn Error>> {
    let device_id = quest_peripheral.id.clone();
    let proj_dirs = ProjectDirs::from("com", "veygax", "hzospal")
        .ok_or("Could not determine config directory")?;
    let config_dir = proj_dirs.config_dir();

    if !config_dir.exists() {
        std::fs::create_dir_all(config_dir)?;
    }

    let device_keys_dir = config_dir.join("device_keys");
    if !device_keys_dir.exists() {
        std::fs::create_dir_all(&device_keys_dir)?;
    }

    let key_path = device_keys_dir.join(format!("{}.bin", sanitize_key_filename(&device_id)));
    let legacy_key_path = config_dir.join("device_key.bin");
    let had_device_key = key_path.exists();

    let device_key = if had_device_key {
        Some(read_device_key(&key_path)?)
    } else if legacy_key_path.exists() {
        Some(read_device_key(&legacy_key_path)?)
    } else {
        None
    };

    let quest = connect_quest_peripheral(quest_peripheral, device_key).await?;

    let mut saved_new_key = false;
    if !had_device_key {
        if let Some(key) = quest.device_key {
            std::fs::write(&key_path, key)?;
            saved_new_key = true;
        }
    }

    Ok(ConnectedQuest {
        device: quest,
        saved_new_key,
        key_path,
    })
}

fn read_device_key(path: &std::path::Path) -> Result<[u8; 32], Box<dyn Error>> {
    let key_vec = std::fs::read(path)?;
    key_vec
        .try_into()
        .map_err(|_| format!("{} is not 32 bytes", path.display()).into())
}

fn sanitize_key_filename(device_id: &str) -> String {
    let mut sanitized = device_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        sanitized.push_str("unknown");
    }

    sanitized
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("hzospal=info,warn"),
    )
    .target(env_logger::Target::Stdout)
    .init();

    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    spawn_quest_worker(command_rx, event_tx);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([840.0, 620.0])
            .with_min_inner_size([520.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "hzospal",
        native_options,
        Box::new(move |cc| Ok(Box::new(HzosPalApp::new(command_tx, event_rx, cc)))),
    )
}
