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
    /// Abre o seletor para uma figurinha nova. O nome vem depois.
    PickSticker,
    /// Sobe a figurinha já escolhida, agora com nome.
    CreateSticker {
        name: String,
        blob: String,
        format: String,
    },
    DeleteEmoji(String),
    LoadAuditLogs,
}
