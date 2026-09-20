//! Conta e servidor: o vocabulário de pedidos da folha de ajustes.
//!
//! As telas vivem em [`super::settings`]. Aqui fica só o que elas pedem, que
//! o `app` traduz em comandos de rede.

use crate::api::models::{UpdateServerRequest, UpdateUserRequest};

#[derive(Clone, Debug)]
pub enum AdminAction {
    SaveProfile(Box<UpdateUserRequest>),
    SetPresence(Option<String>),
    PickAvatar,
    ChangePassword(String),
    LoadDevices,
    DropConnection(String),
    SaveServer(Box<UpdateServerRequest>),
    /// Figurinha nova, com o nome que ela vai ter.
    PickEmoji(String),
    DeleteEmoji(String),
    LoadAuditLogs,
}
