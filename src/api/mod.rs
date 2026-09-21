//! API do núcleo compartilhado.
///
/// O desktop continua usando `crate::api::...`, mas a implementação mora em
/// `papo-core` para Android/iOS ou outra UI poderem ligar ao mesmo cliente.
pub use papo_core::api::{client, models, net, ws};
