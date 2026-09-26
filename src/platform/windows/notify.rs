//! Native Windows toast notifications.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::mpsc;

use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

pub const AUMID: &str = crate::APP_ID;

#[derive(Clone, Debug)]
pub struct Notification {
    pub summary: String,
    pub body: String,
    pub tag: Option<String>,
}

#[derive(Clone)]
pub struct Notifier {
    requests: mpsc::Sender<Notification>,
}

impl Notifier {
    pub fn spawn(repaint: egui::Context) -> Option<Self> {
        if let Err(error) = register_identity() {
            log::warn!("identidade de notificação do Windows indisponível: {error}");
        }

        let (tx, rx) = mpsc::channel::<Notification>();
        std::thread::Builder::new()
            .name("papo-notify".into())
            .spawn(move || {
                if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                    log::warn!("WinRT de notificações indisponível: {error}");
                    return;
                }
                let notifier =
                    match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
                    {
                        Ok(notifier) => notifier,
                        Err(error) => {
                            log::warn!("sem notificações do Windows: {error}");
                            return;
                        }
                    };

                let mut live = HashMap::<String, ToastNotification>::new();
                while let Ok(notification) = rx.recv() {
                    match show(&notifier, &notification, &repaint) {
                        Ok((tag, toast)) => {
                            live.insert(tag, toast);
                        }
                        Err(error) => log::warn!("notificação do Windows recusada: {error}"),
                    }
                }
            })
            .ok()?;

        Some(Self { requests: tx })
    }

    pub fn show(&self, notification: Notification) {
        let _ = self.requests.send(notification);
    }
}

fn show(
    notifier: &windows::UI::Notifications::ToastNotifier,
    notification: &Notification,
    repaint: &egui::Context,
) -> windows::core::Result<(String, ToastNotification)> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(
        r#"<toast><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>"#,
    ))?;

    let texts = document.GetElementsByTagName(&HSTRING::from("text"))?;
    let title = texts.Item(0)?;
    title.AppendChild(&document.CreateTextNode(&HSTRING::from(&notification.summary))?)?;
    let body = texts.Item(1)?;
    body.AppendChild(&document.CreateTextNode(&HSTRING::from(&notification.body))?)?;

    let toast = ToastNotification::CreateToastNotification(&document)?;
    let tag = notification
        .tag
        .as_deref()
        .map(stable_tag)
        .unwrap_or_else(|| "papo".to_owned());
    toast.SetTag(&HSTRING::from(&tag))?;

    let repaint = repaint.clone();
    toast.Activated(&TypedEventHandler::new(move |_, _| {
        repaint.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        repaint.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        repaint.send_viewport_cmd(egui::ViewportCommand::Focus);
        repaint.request_repaint();
        Ok(())
    }))?;

    notifier.Show(&toast)?;
    Ok((tag, toast))
}

fn stable_tag(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("papo-{hash:016x}")
}

/// Register enough per-user identity for unpackaged Win32 toast delivery.
/// The installer also stamps this AUMID onto its shortcuts.
fn register_identity() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW,
        RegSetValueExW,
    };

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let subkey = wide(&format!(r"Software\Classes\AppUserModelId\{AUMID}"));
    let mut key = HKEY::default();
    let status = unsafe {
        RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), &mut key)
    };
    if status.0 != 0 {
        return Err(format!("RegCreateKeyW={}", status.0));
    }

    let set = |name: &str, value: &str| {
        let name = wide(name);
        let value = wide(value);
        let bytes = unsafe {
            std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2)
        };
        unsafe {
            RegSetValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                REG_SZ,
                Some(bytes),
            )
        }
    };

    let display = set("DisplayName", "Papo");
    let icon = std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let icon_status = if icon.is_empty() {
        windows::Win32::Foundation::WIN32_ERROR(0)
    } else {
        set("IconUri", &icon)
    };
    unsafe { let _ = RegCloseKey(key); }

    if display.0 != 0 || icon_status.0 != 0 {
        return Err(format!(
            "RegSetValueExW={}/{}",
            display.0, icon_status.0
        ));
    }
    Ok(())
}
