//! A call vista pela janela: quem está na sala, quem está falando, e onde a
//! call aparece na tela.
//!
//! Isto é só o retrato — o transporte está em `voice`. Os dois são
//! alimentados pelos mesmos eventos do socket: aqui para desenhar, lá para
//! decidir o que pedir ao servidor.
//!
//! O servidor não tem como listar quem já está numa sala de voz: o
//! `voice_joined` só chega para quem entra. O que dá para saber de fora é o
//! que passou pelo socket desde que a janela abriu — é o bastante para a
//! coluna da esquerda mostrar a sala enchendo, e é o que Discord mostraria
//! também depois de um recarregar.

use std::collections::HashMap;

use crate::api::ws::VoiceMember;

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
    /// Quem está em cada canal de voz, pelo que vimos pelo socket.
    pub rooms: HashMap<String, Vec<VoiceMember>>,
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
        "voice-room-closed" | "voice-forbidden" | "voice-invalid-sdp"
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
        } else if self.floating {
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
        let channel_id = std::mem::take(&mut self.channel_id);
        self.rooms.remove(&channel_id);
        self.phase = Phase::Off;
        self.speakers.clear();
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
        self.speakers.retain(|speaker| speaker != user_id);
    }

    pub fn speaking(&self, user_id: &str) -> bool {
        self.speakers.iter().any(|speaker| speaker == user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, camera: bool) -> VoiceMember {
        VoiceMember {
            user_id: id.to_owned(),
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
}
