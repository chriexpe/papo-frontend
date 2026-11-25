//! The image editor (`docs/design.md` §14.10), one dialog wherever Papo
//! takes a picture: a tray tile, the profile photo and banner. The crop's
//! geometry is `crop`'s, the dialog's state `state`'s.

pub(crate) mod crop;
pub(crate) mod cut;
mod stage;
pub(crate) mod state;
pub(crate) mod view;
