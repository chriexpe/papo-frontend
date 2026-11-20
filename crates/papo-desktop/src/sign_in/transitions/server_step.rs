//! The server step's transitions: the probe's answer, the already-added
//! line's Open, and moving on to a server's account steps.

use rune::machine::Effects;

use super::focus_form;
use crate::sign_in::{
    AccountForm, AddressCheck, Discovery, Effect, Fields, Form, Line, Phase, Probe, ProbeFailure,
    SavedAccounts, Screen, Server, SignIn, address, renewing_screen,
};

impl SignIn {
    /// The already-added line's Open: the app when an account works, signing
    /// in again when none does, else the server's account steps.
    pub(super) fn open(&mut self, fields: &mut Fields) -> Effects<Effect> {
        let Screen::Address {
            phase: Phase::Editing(Some(Line::AlreadyAdded(added))),
            ..
        } = &self.screen
        else {
            return Effects::none();
        };
        let added = added.as_ref().clone();
        let mut effects = match added.saved {
            SavedAccounts::Working => return Effects::one(Effect::OpenServer(added.id)),
            SavedAccounts::SignedOut(account) => {
                self.auth = None;
                self.screen = renewing_screen(&account, fields);
                if let Screen::Account { server, .. } = &mut self.screen {
                    server.name = added.name;
                }
                Effects::none()
            }
            SavedAccounts::None => {
                let server = Server {
                    id: added.id,
                    name: added.name,
                    address: added.address,
                    icon: None,
                    unlocked: false,
                    renewing: None,
                    adding: false,
                };
                self.enter_account(fields, server, added.public)
            }
        };
        if let Some(icon) = added.icon {
            effects.push(Effect::LoadIcon(icon));
        }
        effects
    }

    pub(super) fn probed(&mut self, fields: &mut Fields, probe: Probe) -> Effects<Effect> {
        let Screen::Address { phase, .. } = &mut self.screen else {
            return Effects::none();
        };
        match probe {
            Probe::Failed(failure) => {
                *phase = Phase::Editing(Some(match failure {
                    ProbeFailure::Unreachable => Line::Unreachable,
                    ProbeFailure::NotSetUp => Line::NotSetUp,
                    ProbeFailure::NotPapo => Line::NotPapo,
                }));
                // The field took no input while busy; it gets the keyboard back.
                fields.address.focused = true;
                Effects::none()
            }
            Probe::AlreadyAdded(added) => {
                *phase = Phase::Editing(Some(Line::AlreadyAdded(Box::new(added))));
                fields.address.focused = true;
                Effects::none()
            }
            Probe::Added { id, discovery } => {
                let address = match &fields.address_check {
                    AddressCheck::Valid { address, .. } => address::shown(address).to_owned(),
                    AddressCheck::Empty | AddressCheck::Invalid(_) => String::new(),
                };
                let server = Server {
                    id,
                    name: discovery.name.unwrap_or_else(|| address.clone()),
                    address,
                    icon: None,
                    unlocked: false,
                    renewing: None,
                    adding: false,
                };
                let mut effects = self.enter_account(fields, server, discovery.public);
                if let Some(icon) = discovery.icon {
                    effects.push(Effect::LoadIcon(icon));
                }
                effects
            }
        }
    }

    /// Moves to `server`'s account steps: its password first unless it is
    /// public.
    fn enter_account(
        &mut self,
        fields: &mut Fields,
        server: Server,
        public: bool,
    ) -> Effects<Effect> {
        let form = if public {
            Form::Account(AccountForm::SignIn)
        } else {
            Form::Unlock {
                then: AccountForm::SignIn,
                lapsed: false,
            }
        };
        self.auth = None;
        self.screen = Screen::Account {
            server,
            form,
            phase: Phase::Editing(None),
        };
        focus_form(fields, form);
        Effects::none()
    }

    pub(super) fn discovered(&mut self, discovery: Option<Discovery>) -> Effects<Effect> {
        let Screen::Account { server, .. } = &mut self.screen else {
            return Effects::none();
        };
        let Some(discovery) = discovery else {
            return Effects::none();
        };
        if let Some(name) = discovery.name {
            server.name = name;
        }
        discovery
            .icon
            .map_or_else(Effects::none, |icon| Effects::one(Effect::LoadIcon(icon)))
    }
}
