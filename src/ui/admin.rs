//! Conta e servidor: o vocabulário de pedidos da folha de ajustes.
//!
//! As telas vivem em [`super::settings`]. Aqui fica só o que elas pedem, que
//! o `app` traduz em comandos de rede.

use crate::api::models::{PatchServerRequest, UpdateUserRequest};

#[derive(Clone, Debug)]
pub enum AdminAction {
    SaveProfile(Box<UpdateUserRequest>),
    SetPresence(Option<String>),
    PickAvatar,
    /// Abre o seletor para o banner do perfil (3:1).
    PickBanner,
    RemoveBanner,
    /// Abre o seletor para o ícone do servidor.
    PickServerIcon,
    ChangePassword(String),
    LoadDevices,
    DropConnection(String),
    SaveServer(Box<PatchServerRequest>),
    /// Abre o seletor para uma figurinha nova. O nome vem depois.
    PickSticker,
    /// Sobe a figurinha já escolhida, agora com nome.
    CreateSticker {
        name: String,
        blob: String,
        format: String,
    },
    DeleteEmoji(String),
    LoadAuditLogs(crate::api::models::AuditQuery),
}
