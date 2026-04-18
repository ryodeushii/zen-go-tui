use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
#[cfg(target_os = "linux")]
use gtk::prelude::*;
use image::RgbaImage;
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone)]
pub enum TrayEvent {
    ShowWindow,
    ToggleMonitorMute,
    ToggleHp1Mute,
    ToggleHp2Mute,
    Quit,
}

#[derive(Debug, Clone)]
enum TrayCommand {
    SetMonitorMuted(bool),
    SetHp1Muted(bool),
    SetHp2Muted(bool),
}

pub struct TrayHandle {
    command_tx: Sender<TrayCommand>,
}

impl TrayHandle {
    pub fn set_monitor_muted(&self, muted: bool) {
        let _ = self.command_tx.send(TrayCommand::SetMonitorMuted(muted));
    }

    pub fn set_hp1_muted(&self, muted: bool) {
        let _ = self.command_tx.send(TrayCommand::SetHp1Muted(muted));
    }

    pub fn set_hp2_muted(&self, muted: bool) {
        let _ = self.command_tx.send(TrayCommand::SetHp2Muted(muted));
    }
}

pub fn spawn_tray(running: Arc<AtomicBool>) -> (TrayHandle, Receiver<TrayEvent>) {
    let (event_tx, event_rx) = crossbeam_channel::unbounded();
    let (command_tx, command_rx) = crossbeam_channel::unbounded();

    let event_tx_clone = event_tx.clone();

    std::thread::spawn(move || {
        #[cfg(target_os = "linux")]
        init_linux_gtk();

        let tray_menu = Menu::new();

        let show_item = MenuItem::new("Show Control Panel", true, None);
        let separator = PredefinedMenuItem::separator();
        let monitor_item = CheckMenuItem::new("Mute Monitor", true, false, None);
        let hp1_item = CheckMenuItem::new("Mute HP1", true, false, None);
        let hp2_item = CheckMenuItem::new("Mute HP2", true, false, None);
        let quit_item = MenuItem::new("Quit", true, None);

        tray_menu
            .append_items(&[
                &show_item,
                &separator,
                &monitor_item,
                &hp1_item,
                &hp2_item,
                &separator,
                &quit_item,
            ])
            .expect("Failed to build tray menu");

        let icon = create_default_icon();

        let _tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("Zen Go Control Panel")
            .with_icon(icon)
            .build()
            .expect("Failed to create tray icon");

        let menu_event_rx = MenuEvent::receiver();
        let tray_event_rx = TrayIconEvent::receiver();

        while running.load(Ordering::SeqCst) {
            while let Ok(command) = command_rx.try_recv() {
                match command {
                    TrayCommand::SetMonitorMuted(muted) => monitor_item.set_checked(muted),
                    TrayCommand::SetHp1Muted(muted) => hp1_item.set_checked(muted),
                    TrayCommand::SetHp2Muted(muted) => hp2_item.set_checked(muted),
                }
            }

            if let Ok(event) = menu_event_rx.try_recv() {
                if event.id == show_item.id() {
                    let _ = event_tx_clone.send(TrayEvent::ShowWindow);
                } else if event.id == monitor_item.id() {
                    let _ = event_tx_clone.send(TrayEvent::ToggleMonitorMute);
                } else if event.id == hp1_item.id() {
                    let _ = event_tx_clone.send(TrayEvent::ToggleHp1Mute);
                } else if event.id == hp2_item.id() {
                    let _ = event_tx_clone.send(TrayEvent::ToggleHp2Mute);
                } else if event.id == quit_item.id() {
                    let _ = event_tx_clone.send(TrayEvent::Quit);
                    running.store(false, Ordering::SeqCst);
                    break;
                }
            }

            if let Ok(event) = tray_event_rx.try_recv() {
                if let TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    ..
                } = event
                {
                    let _ = event_tx_clone.send(TrayEvent::ShowWindow);
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    (TrayHandle { command_tx }, event_rx)
}

#[cfg(target_os = "linux")]
fn init_linux_gtk() {
    if !gtk::is_initialized() {
        gtk::init().expect("failed to initialize GTK for tray icon");
    }
}

fn create_default_icon() -> tray_icon::Icon {
    let size = 32;
    let mut img = RgbaImage::new(size, size);

    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let cx = size as f32 / 2.0;
        let cy = size as f32 / 2.0;
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist < cx - 2.0 {
            let intensity = (1.0 - dist / cx) * 255.0;
            *pixel = image::Rgba([
                (intensity * 0.29) as u8,
                (intensity * 0.62) as u8,
                (intensity * 1.0) as u8,
                255,
            ]);
        } else {
            *pixel = image::Rgba([0, 0, 0, 0]);
        }
    }

    let dyn_img = image::DynamicImage::ImageRgba8(img);
    let rgba = dyn_img.to_rgba8();
    let (w, h) = rgba.dimensions();

    tray_icon::Icon::from_rgba(rgba.into_raw(), w, h).expect("Failed to create tray icon from RGBA")
}
