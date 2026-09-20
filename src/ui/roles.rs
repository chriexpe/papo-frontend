//! Cargos: o que a folha de ajustes precisa saber sobre eles.
//!
//! A tela em si vive em [`super::settings`], no painel "Cargos" da folha do
//! servidor. Aqui ficam só o rascunho em edição e o vocabulário de pedidos,
//! que o `app` traduz em comandos de rede.

use crate::api::models::RolePermissions;

/// O que a tela pediu.
#[derive(Clone, Debug)]
pub enum RoleAction {
    Create {
        name: String,
        color: Option<String>,
        permissions: RolePermissions,
    },
    Update {
        role_id: String,
        name: String,
        color: Option<String>,
        permissions: RolePermissions,
    },
    Delete(String),
    Assign {
        user_id: String,
        role_id: String,
    },
    Unassign {
        user_id: String,
        role_id: String,
    },
}

/// Cargo em edição. Só vira pedido quando se clica em salvar: marcar cinco
/// permissões não pode ser cinco viagens ao servidor.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    /// `None` enquanto o cargo ainda não existe no servidor.
    pub id: Option<String>,
    pub name: String,
    pub color: String,
    pub permissions: RolePermissions,
}

#[derive(Default)]
pub struct RolesState {
    pub draft: Option<Draft>,
}
