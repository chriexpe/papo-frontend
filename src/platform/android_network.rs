//! Ponte pequena entre ConnectivityManager/lifecycle e os runtimes de rede.
//!
//! O Android conhece somente o caminho de rede do processo. Cada `Net`
//! continua decidindo se o seu servidor é alcançável e a Store continua sendo
//! a autoridade de freshness/generation.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::api::net::{
    Command as NetCommand, NetSender, NetworkAvailability, NetworkHint,
};

#[derive(Default)]
struct Bridge {
    current: NetworkHint,
    next_id: u64,
    senders: HashMap<u64, NetSender>,
}

impl Bridge {
    fn register(&mut self, sender: NetSender) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        sender.send(NetCommand::NetworkHint(self.current));
        self.senders.insert(id, sender);
        id
    }

    fn unregister(&mut self, id: u64) {
        self.senders.remove(&id);
    }

    fn publish(&mut self, next: NetworkHint) {
        if self.current == next {
            return;
        }
        self.current = next;
        let senders: Vec<_> = self.senders.values().cloned().collect();
        for sender in senders {
            sender.send(NetCommand::NetworkHint(next));
        }
    }

    fn resume(&self) {
        let senders: Vec<_> = self.senders.values().cloned().collect();
        for sender in senders {
            sender.send(NetCommand::ProbeConnection);
        }
    }
}

static BRIDGE: LazyLock<Mutex<Bridge>> = LazyLock::new(|| Mutex::new(Bridge::default()));

/// Registro RAII de um runtime. Ao remover/reabrir um Workspace, o sender
/// deixa de receber sinais sem precisar de bookkeeping paralelo no PapoApp.
pub struct Registration {
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut bridge) = BRIDGE.lock() {
            bridge.unregister(self.id);
        }
    }
}

pub fn register(sender: NetSender) -> Registration {
    let id = BRIDGE
        .lock()
        .map(|mut bridge| bridge.register(sender))
        .unwrap_or(u64::MAX);
    Registration { id }
}

/// O foreground é um probe, não uma quebra de continuidade: socket saudável
/// fica Online; zumbi/backoff é descoberto/antecipado pelo runtime normal.
pub fn resumed() {
    if let Ok(bridge) = BRIDGE.lock() {
        bridge.resume();
    }
}

/// Snapshot do Android. `epoch` muda somente quando a identidade do default
/// network muda ou quando ele some/volta. VALIDATED não participa da decisão:
/// uma rede Wi-Fi local pode alcançar perfeitamente um Papo na LAN.
fn network_changed(available: bool, epoch: u64, transport: &str) {
    let hint = NetworkHint {
        availability: if available {
            NetworkAvailability::Available
        } else {
            NetworkAvailability::Unavailable
        },
        epoch,
    };

    let changed = BRIDGE
        .lock()
        .map(|mut bridge| {
            if bridge.current == hint {
                false
            } else {
                bridge.publish(hint);
                true
            }
        })
        .unwrap_or(false);

    if changed {
        if available {
            log::info!("android network: available epoch={epoch} transport={transport}");
        } else {
            log::info!("android network: unavailable epoch={epoch}");
        }
    } else {
        log::debug!("android network: duplicate path callback deduplicated epoch={epoch}");
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeNetworkChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    available: bool,
    epoch: i64,
    transport: jni::objects::JString,
) {
    let transport = env
        .get_string(&transport)
        .map(|value| String::from(value))
        .unwrap_or_else(|_| "other".to_owned());
    network_changed(available, epoch.max(0) as u64, &transport);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(availability: NetworkAvailability, epoch: u64) -> NetworkHint {
        NetworkHint { availability, epoch }
    }

    #[test]
    fn callback_reaches_runtime_without_ui_frame_and_fans_out() {
        let mut bridge = Bridge::default();
        let (a, mut a_rx) = NetSender::channel();
        let (b, mut b_rx) = NetSender::channel();
        bridge.register(a);
        bridge.register(b);

        // Initial Unknown snapshot.
        assert!(matches!(a_rx.try_recv(), Ok(NetCommand::NetworkHint(_))));
        assert!(matches!(b_rx.try_recv(), Ok(NetCommand::NetworkHint(_))));

        let next = hint(NetworkAvailability::Unavailable, 1);
        bridge.publish(next);
        assert!(matches!(
            a_rx.try_recv(),
            Ok(NetCommand::NetworkHint(value)) if value == next
        ));
        assert!(matches!(
            b_rx.try_recv(),
            Ok(NetCommand::NetworkHint(value)) if value == next
        ));
    }

    #[test]
    fn new_runtime_receives_current_snapshot_immediately() {
        let mut bridge = Bridge::default();
        let current = hint(NetworkAvailability::Unavailable, 9);
        bridge.publish(current);

        let (sender, mut rx) = NetSender::channel();
        bridge.register(sender);
        assert!(matches!(
            rx.try_recv(),
            Ok(NetCommand::NetworkHint(value)) if value == current
        ));
    }

    #[test]
    fn removed_runtime_no_longer_receives_hints() {
        let mut bridge = Bridge::default();
        let (sender, mut rx) = NetSender::channel();
        let id = bridge.register(sender);
        let _ = rx.try_recv();

        bridge.unregister(id);
        bridge.publish(hint(NetworkAvailability::Available, 2));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn same_snapshot_is_deduplicated() {
        let mut bridge = Bridge::default();
        let (sender, mut rx) = NetSender::channel();
        bridge.register(sender);
        let _ = rx.try_recv();

        let current = hint(NetworkAvailability::Available, 4);
        bridge.publish(current);
        assert!(rx.try_recv().is_ok());
        bridge.publish(current);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn resume_probes_registered_runtimes_without_rendering() {
        let mut bridge = Bridge::default();
        let (sender, mut rx) = NetSender::channel();
        bridge.register(sender);
        let _ = rx.try_recv();

        bridge.resume();
        assert!(matches!(rx.try_recv(), Ok(NetCommand::ProbeConnection)));
    }

    #[test]
    fn validation_is_not_part_of_runtime_hint() {
        // The generic model deliberately contains only path existence/epoch.
        // Android VALIDATED can therefore never disable LAN-only servers.
        let local_wifi = hint(NetworkAvailability::Available, 3);
        assert_eq!(local_wifi.availability, NetworkAvailability::Available);
    }
}
