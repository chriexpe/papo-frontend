//! Shared audit filtering and sentences; egui supplies its selected language.
pub use papo_core::audit::{Kind, Period, Filter, Piece, Names, deleted_text, is_danger};

pub fn sentence(entry:&crate::api::models::AuditLogEntry,lang:crate::i18n::Lang,names:&Names<'_>)->Vec<Piece> {
    papo_core::audit::sentence(entry,if lang==crate::i18n::Lang::PtBr {papo_core::audit::AuditLanguage::Portuguese} else {papo_core::audit::AuditLanguage::English},names)
}

pub fn day_label(at:chrono::DateTime<chrono::Utc>,now:chrono::DateTime<chrono::Local>,lang:crate::i18n::Lang)->String {
    papo_core::audit::day_label(at,now,if lang==crate::i18n::Lang::PtBr {papo_core::audit::AuditLanguage::Portuguese} else {papo_core::audit::AuditLanguage::English})
}
