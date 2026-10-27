//! egui call orchestration. Protocol guards are shared with the Compose adapter.
use super::*;

/// O que da rede é da call: os servidores ICE, que a fazem nascer, e a
/// sinalização, que vai direto para a thread dela sem passar pelo estado da
/// tela.
pub(super) fn route_call_effect(
    ws: &mut Workspace,
    effect: CallRuntimeEffect,
    ctx: &egui::Context,
) {
    use crate::api::ws::Event;

    match effect {
        CallRuntimeEffect::VoiceReady {
            channel_id,
            attempt,
            servers,
        } => {
            if !ws.runtime.store.call.current(&channel_id, attempt)
                || ws.call_owner != ws.runtime.store.me
                || ws.leave_barrier.waiting()
            {
                return;
            }
            ws.call = Call::start(
                channel_id.clone(),
                IceConfig::from_servers(&servers),
                ctx.clone(),
                ws.runtime.net.sender(),
            );
            ws.call_ready = false;
            if let Some(call) = &ws.call {
                let names = ws
                    .runtime
                    .store
                    .members
                    .iter()
                    .map(|member| (member.id.clone(), member.name.clone()))
                    .collect();
                ws.runtime.net.set_event_callback(Some(
                    call.event_callback(ws.runtime.store.me.clone(), names),
                ));
                // ICE discovery is cancellable; only the current attempt may enter the room.
                ws.runtime.net.send(Command::VoiceSignal(
                    serde_json::json!({
                        "type": "voice_join", "channel_id": channel_id,
                    })
                    .to_string(),
                ));
                #[cfg(target_os = "android")]
                crate::platform::android_call::bind(
                    call.command_sender(),
                    ws.runtime.net.sender(),
                    channel_id,
                    ws.runtime.store.call.muted,
                    ws.runtime.store.call.camera,
                    ws.runtime.store.me.clone(),
                );
            } else {
                ws.runtime.store.call.error = Some("a call não abriu".to_owned());
                ws.runtime.store.call.left();
                #[cfg(target_os = "android")]
                crate::platform::android_call::stop_service();
            }
        }
        CallRuntimeEffect::Event {
            event,
            me_before,
            phase_before,
            store_error_before,
            call_error_before,
        } => {
            if let Event::VoiceLeft {
                channel_id,
                user_id,
            } = event.as_ref()
                && let Some((channel, owner)) = ws.leave_barrier.acknowledge(channel_id, user_id)
                && owner == ws.runtime.store.me
                && ws.runtime.store.connection == crate::api::ws::Connection::Online
            {
                begin_call(ws, channel);
            }
            let Some(call) = &ws.call else { return };
            let mut over = false;
            let mut tell_server = false;
            match *event {
                Event::VoiceLeft {
                    channel_id,
                    user_id,
                } if channel_id == call.channel_id && user_id == me_before => over = true,
                Event::Failure { code, .. }
                    if code.as_deref().is_some_and(|code| {
                        crate::state::call::fatal(code, phase_before == Phase::Joining)
                    }) =>
                {
                    over = true;
                    tell_server = phase_before == Phase::In;
                }
                _ => {}
            }
            if over {
                if tell_server {
                    let channel = call.channel_id.clone();
                    ws.leave_barrier.begin(channel.clone(), me_before.clone());
                    ws.runtime.net.send(Command::VoiceSignal(
                        serde_json::json!({"type":"voice_leave", "channel_id":channel}).to_string(),
                    ));
                    // Before the runtime split, this teardown happened before
                    // Store::apply. Restore the pre-update error projection so
                    // a structural refactor does not change call UX.
                    ws.runtime.store.error = store_error_before;
                    ws.runtime.store.call.error = call_error_before;
                    ws.camera_revision = 0;
                }
                ws.runtime.net.set_event_callback(None);
                ws.call = None;
                ws.call_ready = false;
                ws.watching.clear();
                ws.watching_screens.clear();
                #[cfg(target_os = "android")]
                crate::platform::android_call::stop_service();
            }
        }
        CallRuntimeEffect::ConnectionOffline => {
            ws.leave_barrier.disconnected();
            ws.call_owner.clear();
            ws.runtime.net.set_event_callback(None);
            ws.call = None;
            ws.call_ready = false;
            ws.watching.clear();
            ws.watching_screens.clear();
            ws.runtime.store.call.error = None;
            ws.runtime.store.call.left();
            #[cfg(target_os = "android")]
            crate::platform::android_call::stop_service();
        }
        CallRuntimeEffect::VoiceFailed {
            was_current: true, ..
        } => {
            #[cfg(target_os = "android")]
            crate::platform::android_call::stop_service();
        }
        _ => {}
    }
}

/// Reserve ICE discovery without joining until the attempt is validated by VoiceReady.
pub(super) fn begin_call(ws: &mut Workspace, channel_id: String) {
    if ws.runtime.store.screen != Screen::Chat
        || ws.runtime.store.connection != crate::api::ws::Connection::Online
        || !ws
            .runtime
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.kind == crate::state::ChannelKind::Voice)
    {
        return;
    }
    ws.call_owner = ws.runtime.store.me.clone();
    ws.runtime.store.call.joining(channel_id.clone());
    let attempt = papo_core::state::call::next_attempt();
    ws.runtime.store.call.attempt = attempt;
    ws.runtime.net.send(Command::FetchVoiceIce {
        channel_id,
        attempt,
    });
}

/// Remove only our presence; other occupants remain visible after leaving.
pub(super) fn leave_call(ws: &mut Workspace) {
    ws.leave_barrier.cancel_queued();
    #[cfg(target_os = "android")]
    let had_call = ws.runtime.store.call.active() || ws.call.is_some();
    // Merely fetching ICE never entered the room and must not send a leave.
    if ws.call.is_some() && ws.runtime.store.call.active() && ws.call_owner == ws.runtime.store.me {
        let channel = ws.runtime.store.call.channel_id.clone();
        ws.leave_barrier
            .begin(channel.clone(), ws.call_owner.clone());
        ws.runtime.net.send(Command::VoiceSignal(
            serde_json::json!({
                "type": "voice_leave", "channel_id": channel,
            })
            .to_string(),
        ));
    }
    ws.runtime.net.set_event_callback(None);
    ws.call = None;
    ws.call_ready = false;
    ws.watching.clear();
    ws.watching_screens.clear();
    ws.camera_revision = 0;
    ws.call_owner.clear();
    let channel = ws.runtime.store.call.channel_id.clone();
    ws.runtime.store.call.remove(&channel, &ws.runtime.store.me);
    ws.runtime.store.call.left();
    #[cfg(target_os = "android")]
    if had_call {
        crate::platform::android_call::stop_service();
    }
}

/// O vaivém da call a cada quadro. SDP/ICE já circulam diretamente entre a
/// thread de rede e a thread da call; aqui ficam apenas estado visual e
/// seleção das câmeras que queremos receber.
pub(super) fn pump_call(ws: &mut Workspace) {
    if ws.leave_barrier.timed_out() && ws.leave_barrier.cancel_queued() {
        ws.runtime.store.error = Some("call-leave-timeout".into());
    }
    if !ws.call_owner.is_empty()
        && (ws.call_owner != ws.runtime.store.me
            || !ws.runtime.store.call.active()
            || !ws
                .runtime
                .store
                .channel(&ws.runtime.store.call.channel_id)
                .is_some_and(|c| c.kind == crate::state::ChannelKind::Voice))
    {
        leave_call(ws);
    }
    let Some(call) = &ws.call else { return };

    // A oferta só pode sair depois do `voice_joined`: antes disso o servidor
    // ainda não tem peer para receber a SDP.
    if !ws.call_ready && ws.runtime.store.call.phase == Phase::In {
        call.ready();
        ws.call_ready = true;
        // O que foi clicado enquanto a call abria vale: entramos mudos, como
        // o servidor assume, mas quem já tinha ligado a câmera não pode ficar
        // com o botão aceso e a câmera parada.
        call.set_muted(ws.runtime.store.call.muted);
        if ws.runtime.store.call.camera {
            call.set_camera(true);
        }
    }

    // Aviso que não derruba a call (sem microfone, sem câmera): aparece uma
    // vez, como os outros recados passageiros da tela.
    if let Some(warning) = call.take_warning() {
        ws.runtime.store.error = Some(warning);
    }
    // O botão da câmera segue o dispositivo, não o clique: onde não há
    // webcam — no Flatpak de hoje, por exemplo — ligar não liga nada, e ele
    // tem de voltar sozinho. O mesmo vale para a câmera que morre no meio.
    //
    // Compara-se a conta de respostas, não o valor: a tentativa que não
    // acha câmera nenhuma termina onde começou, e olhar só o valor não veria
    // resposta — o botão ficaria aceso com câmera nenhuma.
    let (revision, camera) = call.camera_state();
    if ws.camera_revision != revision {
        ws.camera_revision = revision;
        ws.runtime.store.call.camera = camera;
    }

    if call.failed() {
        let message = call.error();
        ws.runtime.store.call.error = message.clone();
        // O mesmo aviso passageiro dos outros erros: quem está na tela
        // precisa saber por que a call sumiu.
        ws.runtime.store.error = message;
        leave_call(ws);
        return;
    }

    // Quem ligou a câmera passa a ser assistido sozinho — a escolha de
    // desenho. Daqui sai só a lista de quem se quer ver; quem decide o que
    // cabe nos seis lugares é a thread da call, que é quem sabe quais estão
    // livres e reaproveita o que vaga.
    let me = ws.runtime.store.me.clone();
    let mut wanted: Vec<String> = ws
        .runtime
        .store
        .call
        .members()
        .iter()
        .filter(|member| member.camera_on && member.user_id != me)
        .map(|member| member.user_id.clone())
        .collect();
    wanted.sort();
    if wanted != ws.watching {
        call.watch(wanted.clone());
        ws.watching = wanted;
    }
    let mut screens: Vec<_> = ws
        .runtime
        .store
        .call
        .members()
        .iter()
        .filter(|member| member.screen_sharing && member.user_id != me)
        .map(|member| member.user_id.clone())
        .collect();
    screens.sort();
    if screens != ws.watching_screens {
        call.send(crate::voice::Command::WatchingScreens(screens.clone()));
        ws.watching_screens = screens;
    }
}

pub(super) fn pending_join(ws: &mut Workspace, ctx: &egui::Context, s: &crate::i18n::Strings) {
    let Some((channel, owner)) = ws.leave_barrier.queued() else {
        return;
    };
    if owner != ws.runtime.store.me {
        return;
    }
    let name = ws
        .runtime
        .store
        .channel(channel)
        .map(|c| c.name.as_str())
        .unwrap_or(channel)
        .to_owned();
    let mut cancel = false;
    egui::Area::new(egui::Id::new("pending-call-replacement"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -90.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{} · {}", s.call_connecting, name));
                    cancel = ui.button(s.cancel).clicked();
                });
            });
        });
    if cancel {
        leave_call(ws);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ws::{Connection, Event, VoiceMember};

    fn workspace(ctx: &egui::Context) -> Workspace {
        let cache = std::sync::Arc::new(ClientDb::open(None));
        let notification = std::sync::Arc::new(NotificationCoordinator::new(cache.clone(), None));
        let mut ws = Workspace::open_with_storage(
            &ServerEntry::new("http://127.0.0.1:9".into()),
            &ReadMarks::new(),
            ctx,
            &cache,
            &notification,
            std::sync::Arc::new(papo_core::storage::MemorySecretStore::default()),
        );
        ws.runtime.store.me = "me".into();
        ws.runtime.store.screen = Screen::Chat;
        ws.runtime.store.connection = Connection::Online;
        ws.runtime.store.channels.push(crate::state::Channel {
            id: "room".into(),
            name: "Room".into(),
            kind: crate::state::ChannelKind::Voice,
            topic: None,
            position: 0,
            permissions: vec![],
            notification_settings: "all".into(),
            parent_id: None,
            unread: false,
            mentions: 0,
        });
        ws
    }

    fn ice(ws: &mut Workspace, ctx: &egui::Context, attempt: u64) {
        route_call_effect(
            ws,
            CallRuntimeEffect::VoiceReady {
                channel_id: "room".into(),
                attempt,
                servers: vec![],
            },
            ctx,
        );
    }

    #[test]
    fn parity_cancelled_discovery_cannot_start_media_or_enter_the_room() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        begin_call(&mut ws, "room".into());
        let attempt = ws.runtime.store.call.attempt;
        leave_call(&mut ws);
        ice(&mut ws, &ctx, attempt);
        assert!(ws.call.is_none());
        assert!(!ws.runtime.store.call.active());
        assert!(
            !ws.leave_barrier.waiting(),
            "discovery did not join, so it owes no leave"
        );
        begin_call(&mut ws, "room".into());
        assert_ne!(attempt, ws.runtime.store.call.attempt);
        ice(&mut ws, &ctx, attempt);
        assert!(ws.call.is_none());
        ws.runtime.store.me = "replacement-account".into();
        let current = ws.runtime.store.call.attempt;
        ice(&mut ws, &ctx, current);
        assert!(ws.call.is_none());
    }

    #[test]
    fn parity_replacement_begins_only_after_leave_reduction_and_retains_other_occupants() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        begin_call(&mut ws, "room".into());
        let old = ws.runtime.store.call.attempt;
        ws.runtime.store.call.joined(
            "room",
            vec![
                VoiceMember {
                    user_id: "me".into(),
                    ..Default::default()
                },
                VoiceMember {
                    user_id: "other".into(),
                    ..Default::default()
                },
            ],
            vec![],
        );
        leave_call(&mut ws);
        assert_eq!(ws.runtime.store.call.room("room").len(), 1);
        ws.leave_barrier.begin("room".into(), "me".into());
        ws.leave_barrier.queue("room".into(), "me".into());
        ice(&mut ws, &ctx, old);
        assert!(ws.call.is_none());
        let event = Event::VoiceLeft {
            channel_id: "room".into(),
            user_id: "me".into(),
        };
        ws.runtime
            .store
            .apply(crate::api::net::Update::Event(Box::new(event.clone())));
        route_call_effect(
            &mut ws,
            CallRuntimeEffect::Event {
                event: Box::new(event),
                me_before: "me".into(),
                phase_before: Phase::Off,
                store_error_before: None,
                call_error_before: None,
            },
            &ctx,
        );
        assert_eq!(ws.runtime.store.call.phase, Phase::Joining);
        assert_ne!(ws.runtime.store.call.attempt, old);
        assert!(!ws.leave_barrier.waiting());
        assert_eq!(ws.runtime.store.call.room("room")[0].user_id, "other");
        ice(&mut ws, &ctx, old);
        assert!(ws.call.is_none());
    }

    #[test]
    fn parity_disconnect_cancels_a_queued_replacement_without_autojoining() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        ws.leave_barrier.begin("room".into(), "me".into());
        ws.leave_barrier.queue("room".into(), "me".into());
        route_call_effect(&mut ws, CallRuntimeEffect::ConnectionOffline, &ctx);
        assert!(!ws.leave_barrier.waiting());
        assert!(!ws.runtime.store.call.active());
        assert!(ws.leave_barrier.acknowledge("room", "me").is_none());
    }
}
