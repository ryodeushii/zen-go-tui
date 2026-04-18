use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use clap::Parser;
use iced::widget::{column, container, scrollable, text};
use iced::{Element, Subscription, Task, Theme};
use crossbeam_channel::Receiver;
use iced_runtime::window;
use iced::window::Id as WindowId;

use zen_go_tui::app::Intent;
use zen_go_tui::transport::{is_device_error, HidTransport, MockTransport, ThreadedTransport, Transport};

mod app;
mod theme;
mod tray;
mod widgets;

use app::GuiApp;
use theme::{ZenTheme, ZEN_THEME};
use tray::{TrayEvent, TrayHandle};

#[derive(Parser, Debug)]
#[command(author, version, about = "Zen Go Synergy Core GUI control panel")]
struct Cli {
    #[arg(long)]
    mock: bool,
}

#[derive(Debug, Clone)]
enum Message {
    DeviceUpdate(Vec<u8>),
    DeviceError(String),
    UserIntent(Intent),
    Tick,
    Tray(TrayEvent),
    WindowCloseRequested(WindowId),
    WindowShowResolved(Option<WindowId>),
    WindowHideResolved(Option<WindowId>),
}

struct State {
    gui: GuiApp,
    transport: Option<Arc<Mutex<Box<dyn Transport>>>>,
    tray: Option<TrayHandle>,
    tray_rx: Receiver<TrayEvent>,
    running: Arc<AtomicBool>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mock = cli.mock;
    let transport = if mock {
        Some(Arc::new(Mutex::new(Box::new(MockTransport::default()) as Box<dyn Transport>)))
    } else {
        match HidTransport::open(0x23e5, 0xa015) {
            Ok(hid) => Some(Arc::new(Mutex::new(
                Box::new(ThreadedTransport::spawn(hid)) as Box<dyn Transport>
            ))),
            Err(e) => {
                eprintln!("Failed to open device: {:#}", e);
                None
            }
        }
    };

    let mut gui = GuiApp::new();
    if transport.is_none() {
        gui.set_error("Device not found. Check USB connection and HID permissions.");
    }

    let running = Arc::new(AtomicBool::new(true));
    let (tray, tray_rx) = tray::spawn_tray(running.clone());

    let state = State {
        gui,
        transport,
        tray: Some(tray),
        tray_rx,
        running,
    };

    iced::application("Zen Go Control Panel", update, view)
        .subscription(subscription)
        .theme(theme)
        .window_size(iced::Size::new(1024.0, 700.0))
        .resizable(true)
        .antialiasing(true)
        .run_with(move || (state, Task::none()))?;

    Ok(())
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::DeviceUpdate(frame) => {
            if frame.len() == 320 {
                let raw: [u8; 320] = frame.try_into().unwrap();
                state.gui.process_frame(raw);
            }
        }
        Message::DeviceError(error) => {
            if is_device_error(&anyhow::anyhow!(error.clone())) {
                state.gui.set_disconnected();
            } else {
                state.gui.set_error(&error);
            }
        }
        Message::UserIntent(intent) => {
            if let Some(transport) = &state.transport {
                if let Err(e) = state.gui.handle_intent(&intent, transport.clone()) {
                    state.gui.set_error(&format!("Command failed: {:#}", e));
                }
            }
        }
        Message::Tick => {
            if let Some(transport) = &state.transport {
                let _ = state.gui.refresh_state(transport.clone());
            }
        }
        Message::Tray(event) => match event {
            TrayEvent::ShowWindow => {
                return window::get_latest().map(Message::WindowShowResolved);
            }
            TrayEvent::ToggleMonitorMute => {
                if let Some(transport) = &state.transport {
                    let _ = state.gui.handle_intent(&Intent::ToggleOutputMute(0), transport.clone());
                }
            }
            TrayEvent::ToggleHp1Mute => {
                if let Some(transport) = &state.transport {
                    let _ = state.gui.handle_intent(&Intent::ToggleOutputMute(1), transport.clone());
                }
            }
            TrayEvent::ToggleHp2Mute => {
                if let Some(transport) = &state.transport {
                    let _ = state.gui.handle_intent(&Intent::ToggleOutputMute(2), transport.clone());
                }
            }
            TrayEvent::Quit => {
                state.running.store(false, Ordering::SeqCst);
                return iced::exit();
            }
        },
        Message::WindowCloseRequested(_id) => {
            return window::get_latest().map(Message::WindowHideResolved);
        }
        Message::WindowShowResolved(Some(id)) => {
            return Task::batch([
                window::minimize::<Message>(id, false),
                window::gain_focus::<Message>(id),
            ]);
        }
        Message::WindowShowResolved(None) => {
            return Task::none();
        }
        Message::WindowHideResolved(Some(id)) => {
            return window::minimize::<Message>(id, true);
        }
        Message::WindowHideResolved(None) => {
            return Task::none();
        }
    }

    sync_tray_state(state);
    Task::none()
}

fn view(state: &State) -> Element<'_, Message> {
    if let Some(error) = state.gui.error_message() {
        return container(
            column![
                text("Error").size(20),
                text(error).size(14),
            ]
            .spacing(10),
        )
        .padding(20)
        .into();
    }

    let content = column![
        widgets::titlebar::view(&state.gui.state, &ZEN_THEME),
        widgets::preamp_bar::view(&state.gui.state, &ZEN_THEME),
        widgets::mixer::view(&state.gui.state, &ZEN_THEME),
        widgets::outputs::view(&state.gui.state, &ZEN_THEME),
    ]
    .spacing(2);

    let scrollable_content = scrollable(content);

    if state.gui.popup_open() {
        container(
            column![
                scrollable_content,
                widgets::popup::view(&state.gui.state, &ZEN_THEME),
            ]
            .spacing(0),
        )
        .padding(4)
        .into()
    } else {
        container(scrollable_content).padding(4).into()
    }
}

fn subscription(state: &State) -> Subscription<Message> {
    let transport = state.transport.clone();
    let tray_rx = state.tray_rx.clone();

    Subscription::batch([
        window::close_requests().map(Message::WindowCloseRequested),
        Subscription::run_with_id(
            "gui-events",
            iced::stream::channel(16, move |mut output| async move {
                loop {
                    while let Ok(event) = tray_rx.try_recv() {
                        let _ = output.try_send(Message::Tray(event));
                    }

                    if let Some(transport) = &transport {
                        match transport.lock() {
                            Ok(t) => {
                                let transport_ref: &dyn Transport = t.as_ref();
                                match transport_ref.read(std::time::Duration::from_millis(100)) {
                                    Ok(Some(frame)) => {
                                        let _ = output.try_send(Message::DeviceUpdate(frame));
                                        continue;
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        let _ = output.try_send(Message::DeviceError(e.to_string()));
                                        continue;
                                    }
                                }
                            }
                            Err(_) => {
                                let _ = output.try_send(Message::DeviceError("Transport lock poisoned".to_string()));
                                continue;
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    let _ = output.try_send(Message::Tick);
                }
            }),
        ),
    ])
}

fn theme(_state: &State) -> Theme {
    Theme::custom("Zen Dark".to_string(), ZenTheme::dark_palette())
}

fn sync_tray_state(state: &State) {
    let Some(tray) = &state.tray else {
        return;
    };

    tray.set_monitor_muted(
        state.gui.state.output.states[0].mode == antelope_protocol::OutputMode::Mute,
    );
    tray.set_hp1_muted(
        state.gui.state.output.states[1].mode == antelope_protocol::OutputMode::Mute,
    );
    tray.set_hp2_muted(
        state.gui.state.output.states[2].mode == antelope_protocol::OutputMode::Mute,
    );
}
