use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use clap::Parser;
use iced::futures::StreamExt;
use iced::widget::{column, container, scrollable, text};
use iced::{Element, Subscription, Task, Theme};

use zen_go_tui::app::Intent;
use zen_go_tui::transport::{is_device_error, HidTransport, MockTransport, ThreadedTransport, Transport};

mod app;
mod theme;
mod widgets;

use app::GuiApp;
use theme::ZenTheme;

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
}

struct State {
    gui: GuiApp,
    transport: Option<Arc<Mutex<Box<dyn Transport>>>>,
    mock: bool,
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

    let state = State { gui, transport, mock };

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
    }
    Task::none()
}

fn view(state: &State) -> Element<'_, Message> {
    let zen_theme = ZenTheme::default();

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
        widgets::titlebar::view(&state.gui.state, &zen_theme),
        widgets::preamp_bar::view(&state.gui.state, &zen_theme),
        widgets::mixer::view(&state.gui.state, &zen_theme),
        widgets::outputs::view(&state.gui.state, &zen_theme),
    ]
    .spacing(2);

    let scrollable_content = scrollable(content);

    if state.gui.popup_open() {
        container(
            column![
                scrollable_content,
                widgets::popup::view(&state.gui.state, &zen_theme),
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
    
    Subscription::run_with_id(
        "device-poll",
        iced::stream::channel(16, move |mut output| async move {
            loop {
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
    )
}

fn theme(_state: &State) -> Theme {
    Theme::custom("Zen Dark".to_string(), ZenTheme::dark_palette())
}