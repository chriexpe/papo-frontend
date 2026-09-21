use crate::api::net::{Command, Net, Wake};
use crate::state::Store;
use crate::storage::SecretStore;
use std::sync::Arc;

/// Cliente Papo compartilhado: transporte e estado autoritativo no mesmo dono.
///
/// Frontends podem enviar comandos e drenar atualizações sem recriar o reducer.
pub struct Core {
    pub net: Net,
    pub state: Store,
}

impl Core {
    pub fn spawn(
        base_url: String,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            net: Net::spawn(base_url, wake, storage),
            state: Store::default(),
        }
    }

    pub fn send(&self, command: Command) {
        self.net.send(command);
    }

    /// Aplica todas as atualizações pendentes e devolve quantas foram processadas.
    pub fn poll(&mut self) -> usize {
        let mut count = 0;
        while let Some(update) = self.net.try_recv() {
            self.state.apply(update);
            count += 1;
        }
        count
    }
}
