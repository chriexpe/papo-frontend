//! Estado compartilhado da aplicação.
///
/// O reducer e os modelos vivem em `papo-core`. Só os dados de demonstração
/// continuam no frontend porque criam mídia/arquivos específicos do desktop.
pub mod demo;

pub use papo_core::server_key;
pub use papo_core::state::*;
pub use papo_core::state::call;
