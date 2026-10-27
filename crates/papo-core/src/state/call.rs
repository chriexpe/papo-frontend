//! A call vista pela janela: quem está na sala, quem está falando, e onde a
//! call aparece na tela.
//!
//! Isto é só o retrato — o transporte está em `voice`. Os dois são
//! alimentados pelos mesmos eventos do socket: aqui para desenhar, lá para
//! decidir o que pedir ao servidor.
//!
//! A lista fria de salas vem de `presence_sync.user_voice`; depois disso,
//! `voice_state_update`/`voice_leave` mantêm o retrato vivo. Ao entrar,
//! `voice_joined` substitui a nossa sala pelo snapshot rico (mute/câmera/tela).

use std::collections::HashMap;

use crate::api::ws::VoiceMember;

pub use crate::next_operation_id as next_attempt;

/// The protocol has no attempt ID on leave events. A timeout cannot release this barrier;
/// only the matching acknowledgement (after Store applies it) or disconnect can do so.
#[derive(Default)]
pub struct LeaveBarrier {
    pending: Option<(String, String, std::time::Instant)>,
    queued: Option<(String, String)>,
}

impl LeaveBarrier {
    pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    pub fn begin(&mut self, channel: String, owner: String) {
        if self.pending.is_none() {
            self.pending = Some((channel, owner, std::time::Instant::now()));
        }
    }

    pub fn queued(&self) -> Option<(&str, &str)> {
        self.queued.as_ref().map(|(c, o)| (c.as_str(), o.as_str()))
    }
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    pub fn timed_out(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, _, since)| since.elapsed() >= Self::TIMEOUT)
    }

    pub fn queue(&mut self, channel: String, owner: String) {
        self.queued = Some((channel, owner));
    }

    pub fn cancel_queued(&mut self) -> bool {
        self.queued.take().is_some()
    }

    pub fn acknowledge(&mut self, channel: &str, owner: &str) -> Option<(String, String)> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|(c, o, _)| c == channel && o == owner)
        {
            return None;
        }
        self.pending = None;
        self.queued.take()
    }

    pub fn disconnected(&mut self) {
        *self = Self::default();
    }
}

/// Em que pé está a nossa entrada na call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Fora de qualquer call.
    #[default]
    Off,
    /// Pedimos para entrar e o servidor ainda não respondeu.
    Joining,
    /// Dentro: o `voice_joined` chegou.
    In,
}

/// Onde a call é desenhada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// No canal, como qualquer outra tela: a grade ocupa a conversa.
    Docked,
    /// Folha de vidro por cima da conversa — é para onde a call vai assim
    /// que aparece vídeo, para o texto continuar alcançável embaixo.
    Sheet,
    /// Mini-overlay da call por cima da conversa. É parte da janela do Papo
    /// e existe igualmente no desktop e no Android.
    Floating,
    /// Janela própria do sistema, para jogar noutro monitor.
    Window,
}

#[derive(Debug, Default)]
pub struct CallState {
    /// Canal da call em que estamos (ou entrando).
    pub channel_id: String,
    pub phase: Phase,
    /// Quem está em cada canal de voz.
    pub rooms: HashMap<String, Vec<VoiceMember>>,
    /// track_id -> user_id dos slots de áudio desta PeerConnection.
    pub audio_routes: HashMap<String, String>,
    /// Older servers do not publish track ownership; participant mixing is unavailable there.
    pub audio_routes_known: bool,
    /// Quem está falando agora, do mais alto para o mais baixo.
    pub speakers: Vec<String>,
    pub muted: bool,
    pub camera: bool,
    /// A folha foi encolhida numa pastilha pelo usuário.
    pub collapsed: bool,
    /// A call está no mini-overlay sobre a conversa.
    pub floating: bool,
    /// A call foi jogada numa janela do sistema só dela.
    pub popped_out: bool,
    /// `has_video` no quadro anterior. Vídeo que aparece ou some com a
    /// pessoa noutro canal muda a apresentação tanto quanto trocar de canal.
    pub had_video: bool,
    pub error: Option<String>,
    /// Qual tentativa de entrada é a atual. O canal não basta para
    /// identificar uma: sair e entrar de novo no mesmo canal dá duas, e a
    /// resposta lenta da primeira chegaria a tempo de derrubar a segunda.
    pub attempt: u64,
}

/// Um erro de voz que acaba com a call, ou só um pedido recusado?
///
/// O servidor usa o mesmo evento `error` para os dois: "você não pode entrar
/// aqui" e "você pediu vídeo demais rápido demais". Derrubar a call no
/// segundo caso deixaria todo mundo mudo por causa de um clique apressado.
pub fn fatal(code: &str, joining: bool) -> bool {
    match code {
        // A sala sumiu, a permissão sumiu, ou a SDP não serve: não há call
        // para continuar.
        //
        // `voice-not-found` fica de fora de propósito, apesar do nome: o
        // servidor devolve o mesmo código quando a *outra* pessoa parou de
        // publicar a câmera entre o nosso pedido e a chegada dele — uma
        // corrida rotineira. Derrubar a call por causa dela deixaria todo
        // mundo mudo porque uma câmera desligou no instante errado. Quando é
        // a nossa sessão que sumiu de verdade, quem avisa é a conexão de
        // mídia caindo.
        "voice-room-closed"
        | "voice-forbidden"
        | "voice-invalid-sdp"
        | "voice-codec-unsupported" => true,
        // Sala cheia e "já está dentro" só falam da entrada. Depois dela,
        // sala cheia é falta de lugar de vídeo — o vídeo é que não vem.
        "voice-room-full" | "voice-already-in-room" => joining,
        _ => false,
    }
}

impl CallState {
    pub fn active(&self) -> bool {
        self.phase != Phase::Off
    }

    /// A sala em que estamos.
    pub fn members(&self) -> &[VoiceMember] {
        self.room(&self.channel_id)
    }

    pub fn room(&self, channel_id: &str) -> &[VoiceMember] {
        self.rooms
            .get(channel_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn seed_presence<I>(&mut self, users: I)
    where
        I: IntoIterator<Item = (String, Vec<String>)>,
    {
        let mut next: HashMap<String, Vec<VoiceMember>> = HashMap::new();
        for (user_id, channels) in users {
            for channel_id in channels {
                let previous = self
                    .rooms
                    .get(&channel_id)
                    .and_then(|room| room.iter().find(|member| member.user_id == user_id))
                    .cloned()
                    .unwrap_or_else(|| VoiceMember {
                        user_id: user_id.clone(),
                        ..Default::default()
                    });
                next.entry(channel_id).or_default().push(previous);
            }
        }
        self.rooms = next;
    }

    pub fn set_audio_routes(
        &mut self,
        channel_id: &str,
        routes: impl IntoIterator<Item = crate::api::ws::VoiceAudioRoute>,
    ) {
        if channel_id != self.channel_id {
            return;
        }
        self.audio_routes_known = true;
        self.audio_routes = routes
            .into_iter()
            .map(|route| (route.track_id, route.user_id))
            .collect();
    }

    /// Tem vídeo de alguém (ou seu) na sala.
    pub fn has_video(&self) -> bool {
        self.camera
            || self
                .members()
                .iter()
                .any(|member| member.camera_on || member.screen_sharing)
    }

    /// Onde desenhar: a call começa no canal e sobe para a folha quando
    /// aparece vídeo — foi a escolha de desenho, e ela também é a prática:
    /// só voz não precisa tomar a tela, vídeo precisa.
    pub fn stage(&self) -> Stage {
        if self.popped_out {
            Stage::Window
        } else if self.floating && self.has_video() {
            Stage::Floating
        } else if self.has_video() && !self.collapsed {
            Stage::Sheet
        } else {
            Stage::Docked
        }
    }

    /// Entrando numa call nova: limpa o que era da anterior e devolve o
    /// número desta tentativa, que volta com as respostas da rede.
    pub fn joining(&mut self, channel_id: String) -> u64 {
        self.attempt = self.attempt.wrapping_add(1);
        self.channel_id = channel_id;
        self.phase = Phase::Joining;
        self.speakers.clear();
        self.audio_routes.clear();
        self.audio_routes_known = false;
        self.muted = true;
        self.camera = false;
        self.collapsed = false;
        self.floating = false;
        self.popped_out = false;
        self.error = None;
        self.attempt
    }

    /// A resposta é desta entrada, ou de uma que já foi abandonada?
    pub fn current(&self, channel_id: &str, attempt: u64) -> bool {
        self.active() && self.attempt == attempt && self.channel_id == channel_id
    }

    pub fn left(&mut self) {
        self.channel_id.clear();
        self.phase = Phase::Off;
        self.speakers.clear();
        self.audio_routes.clear();
        self.audio_routes_known = false;
        self.camera = false;
        self.collapsed = false;
        self.floating = false;
        self.popped_out = false;
    }

    /// Estado inicial da sala, em resposta à nossa entrada.
    pub fn joined(&mut self, channel_id: &str, members: Vec<VoiceMember>, speakers: Vec<String>) {
        if channel_id != self.channel_id {
            return;
        }
        self.phase = Phase::In;
        self.rooms.insert(channel_id.to_owned(), members);
        self.speakers = speakers;
    }

    /// Alguém mudou de estado (ou entrou: o servidor anuncia a entrada com
    /// um estado zerado).
    pub fn update(&mut self, channel_id: &str, state: VoiceMember) {
        let room = self.rooms.entry(channel_id.to_owned()).or_default();
        match room
            .iter_mut()
            .find(|member| member.user_id == state.user_id)
        {
            Some(member) => *member = state,
            None => room.push(state),
        }
    }

    pub fn remove(&mut self, channel_id: &str, user_id: &str) {
        if let Some(room) = self.rooms.get_mut(channel_id) {
            room.retain(|member| member.user_id != user_id);
            if room.is_empty() {
                self.rooms.remove(channel_id);
            }
        }
        if channel_id == self.channel_id {
            self.speakers.retain(|speaker| speaker != user_id);
        }
    }

    pub fn speaking(&self, user_id: &str) -> bool {
        self.speakers.iter().any(|speaker| speaker == user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_replacement_waits_for_its_own_leave_and_can_be_cancelled() {
        let mut barrier = LeaveBarrier::default();
        barrier.begin("room".into(), "me".into());
        barrier.queue("replacement".into(), "me".into());
        assert!(barrier.acknowledge("other-room", "me").is_none());
        assert!(barrier.acknowledge("room", "other-account").is_none());
        assert!(barrier.waiting());
        assert_eq!(
            barrier.acknowledge("room", "me"),
            Some(("replacement".into(), "me".into()))
        );
        assert!(!barrier.waiting());
        barrier.begin("room".into(), "me".into());
        barrier.queue("room".into(), "me".into());
        barrier.cancel_queued();
        assert!(barrier.acknowledge("room", "me").is_none());
        assert!(!barrier.waiting());
    }

    #[test]
    fn parity_timeout_cancels_replacement_but_never_releases_the_leave_barrier() {
        let mut barrier = LeaveBarrier {
            pending: Some((
                "room".into(),
                "me".into(),
                std::time::Instant::now() - LeaveBarrier::TIMEOUT,
            )),
            queued: Some(("room".into(), "me".into())),
        };
        assert!(barrier.timed_out());
        assert!(barrier.cancel_queued());
        assert!(barrier.waiting());
        assert!(barrier.acknowledge("room", "me").is_none());
        assert!(!barrier.waiting());
        barrier.begin("room".into(), "me".into());
        barrier.queue("room".into(), "me".into());
        barrier.disconnected();
        assert!(!barrier.waiting());
        assert!(barrier.acknowledge("room", "me").is_none());
    }

    fn member(id: &str, camera: bool) -> VoiceMember {
        VoiceMember {
            user_id: id.to_owned(),
            state_known: true,
            muted: false,
            camera_on: camera,
            screen_sharing: false,
        }
    }

    #[test]
    fn so_voz_fica_no_canal_e_video_sobe_para_a_folha() {
        let mut call = CallState::default();
        call.joining("c1".to_owned());
        call.joined("c1", vec![member("ana", false)], Vec::new());
        assert_eq!(call.stage(), Stage::Docked);

        call.update("c1", member("ana", true));
        assert_eq!(call.stage(), Stage::Sheet);

        call.floating = true;
        assert_eq!(call.stage(), Stage::Floating);

        // O overlay é apresentação de vídeo, não um retângulo vazio. Se a
        // última câmera some ele volta à forma de voz, mas deixa a intenção
        // de flutuar armada para reaparecer quando o vídeo voltar.
        call.update("c1", member("ana", false));
        assert_eq!(call.stage(), Stage::Docked);
        assert!(call.floating);
        call.update("c1", member("ana", true));
        assert_eq!(call.stage(), Stage::Floating);
        call.floating = false;

        // Encolher devolve a conversa; a call continua.
        call.collapsed = true;
        assert_eq!(call.stage(), Stage::Docked);
        call.popped_out = true;
        assert_eq!(call.stage(), Stage::Window);
    }

    /// Pedido de vídeo recusado não derruba a call; entrada recusada, sim.
    #[test]
    fn so_o_erro_que_acaba_com_a_sala_acaba_com_a_call() {
        assert!(fatal("voice-forbidden", false));
        // Pedir a câmera de quem acabou de desligá-la é corrida, não fim.
        assert!(!fatal("voice-not-found", false));
        assert!(fatal("voice-room-full", true));
        assert!(!fatal("voice-room-full", false));
        assert!(!fatal("voice-rate-limited", false));
    }

    /// Entrar, sair e entrar de novo no mesmo canal são duas tentativas: a
    /// resposta atrasada da primeira não pode mexer na segunda.
    #[test]
    fn resposta_de_uma_entrada_abandonada_nao_vale() {
        let mut call = CallState::default();
        let first = call.joining("c1".to_owned());
        call.left();
        let second = call.joining("c1".to_owned());
        assert!(!call.current("c1", first));
        assert!(call.current("c1", second));
        assert!(!call.current("c2", second));
    }

    #[test]
    fn presence_sync_semeia_salas_e_preserva_estado_rico() {
        let mut call = CallState::default();
        call.update("c1", member("ana", true));
        call.seed_presence([
            ("ana".to_owned(), vec!["c1".to_owned()]),
            ("beto".to_owned(), vec!["c2".to_owned()]),
        ]);
        assert!(call.room("c1")[0].camera_on);
        assert_eq!(call.room("c2")[0].user_id, "beto");
    }

    /// O evento de entrada de alguém é um `voice_state_update` zerado: quem
    /// ainda não está na lista entra nela.
    #[test]
    fn estado_de_quem_chega_cria_a_pessoa_na_sala() {
        let mut call = CallState::default();
        call.joining("c1".to_owned());
        call.update("c1", member("beto", false));
        assert_eq!(call.members().len(), 1);
        call.remove("c1", "beto");
        assert!(call.members().is_empty());
    }
    #[test]
    fn leaving_preserves_other_occupants_and_room_events_are_scoped() {
        let mut call = CallState::default();
        call.joining("c1".into());
        call.joined(
            "c1",
            vec![member("me", false), member("ana", true)],
            vec!["ana".into()],
        );
        call.remove("c2", "ana");
        assert!(call.speaking("ana"));
        call.remove("c1", "me");
        call.left();
        assert_eq!(call.room("c1")[0].user_id, "ana");
        assert!(!call.active());
        assert!(!call.audio_routes_known);
    }

    #[test]
    fn cold_presence_does_not_invent_device_status() {
        let mut call = CallState::default();
        call.seed_presence([("ana".into(), vec!["c1".into()])]);
        assert!(!call.room("c1")[0].state_known);
        call.update("c1", member("ana", true));
        call.seed_presence([("ana".into(), vec!["c1".into()])]);
        assert!(call.room("c1")[0].state_known);
        call.joining("c1".into());
        call.set_audio_routes("c2", []);
        assert!(!call.audio_routes_known);
        call.set_audio_routes("c1", []);
        assert!(call.audio_routes_known);
    }
}
