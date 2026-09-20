//! Os lugares de vídeo do servidor, espelhados aqui.
//!
//! O SFU não renegocia a cada câmera que liga: ele abre N lugares fixos no
//! começo da call e vai trocando quem ocupa cada um. Nenhum evento diz "a
//! Ana está no lugar 2", então o vídeo que chega no lugar 2 não teria nome.
//!
//! A saída é repetir aqui a mesma regra de lá (`findVideoSlotFor`): o lugar
//! de quem já estava, senão o primeiro vazio, sempre na mesma ordem. Como o
//! servidor decide na ordem em que os nossos pedidos chegam, as duas contas
//! dão o mesmo resultado. Se algum dia divergirem, o preço é um nome trocado
//! embaixo de um vídeo — não vídeo nenhum.

/// Quantos lugares de vídeo pedir. É o padrão do backend
/// (`VOICE_VIDEO_SLOTS`); pedir mais não adianta, pedir menos desperdiça.
pub const VIDEO_SLOTS: usize = 6;

/// Quantos lugares de áudio pedir (`VOICE_AUDIO_SLOTS`). O servidor os
/// preenche sozinho com quem está falando — não se pede áudio de ninguém.
pub const AUDIO_SLOTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Camera,
    /// Compartilhar tela. O caminho do vídeo é o mesmo da câmera e o
    /// servidor já trata os dois lados; o que falta é a captura pelo portal
    /// e o lugar dela na grade, que ficaram para a etapa seguinte.
    #[allow(dead_code)]
    Screen,
}

impl Kind {
    /// Como o kind se chama no protocolo.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Camera => "video",
            Self::Screen => "screen",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupant {
    pub publisher: String,
    pub kind: Kind,
}

#[derive(Debug, Default)]
pub struct Slots {
    places: Vec<Option<Occupant>>,
}

impl Slots {
    pub fn new(count: usize) -> Self {
        Self {
            places: vec![None; count],
        }
    }

    /// O lugar que a assinatura vai ocupar, pela regra do servidor: o que já
    /// é dessa pessoa e desse tipo, senão o primeiro vazio. `None` quando a
    /// sala encheu — aí o pedido nem sai.
    pub fn assign(&mut self, publisher: &str, kind: Kind) -> Option<usize> {
        let existing = self.find(publisher, kind);
        let index = existing.or_else(|| self.places.iter().position(Option::is_none))?;
        self.places[index] = Some(Occupant {
            publisher: publisher.to_owned(),
            kind,
        });
        Some(index)
    }

    pub fn find(&self, publisher: &str, kind: Kind) -> Option<usize> {
        self.places.iter().position(|place| {
            place
                .as_ref()
                .is_some_and(|slot| slot.publisher == publisher && slot.kind == kind)
        })
    }

    /// Larga o lugar de uma track (a pessoa desligou a câmera, ou pedimos
    /// para parar de ver).
    pub fn release(&mut self, publisher: &str, kind: Kind) {
        for place in &mut self.places {
            if place
                .as_ref()
                .is_some_and(|slot| slot.publisher == publisher && slot.kind == kind)
            {
                *place = None;
            }
        }
    }

    /// Larga tudo que era de alguém que saiu da call.
    pub fn release_peer(&mut self, publisher: &str) {
        for place in &mut self.places {
            if place.as_ref().is_some_and(|slot| slot.publisher == publisher) {
                *place = None;
            }
        }
    }

    pub fn occupant(&self, index: usize) -> Option<&Occupant> {
        self.places.get(index).and_then(Option::as_ref)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assina_na_ordem_e_reaproveita_o_mesmo_lugar() {
        let mut slots = Slots::new(3);
        assert_eq!(slots.assign("ana", Kind::Camera), Some(0));
        assert_eq!(slots.assign("beto", Kind::Camera), Some(1));
        // A mesma pessoa e o mesmo tipo voltam para o lugar que já era dela.
        assert_eq!(slots.assign("ana", Kind::Camera), Some(0));
        // A tela da Ana é outra track: lugar novo.
        assert_eq!(slots.assign("ana", Kind::Screen), Some(2));
        assert_eq!(slots.assign("clara", Kind::Camera), None);
    }

    #[test]
    fn quem_sai_devolve_os_lugares() {
        let mut slots = Slots::new(2);
        slots.assign("ana", Kind::Camera);
        slots.assign("ana", Kind::Screen);
        slots.release_peer("ana");
        assert_eq!(slots.assign("beto", Kind::Camera), Some(0));
    }

    #[test]
    fn desligar_a_camera_libera_so_a_camera() {
        let mut slots = Slots::new(2);
        slots.assign("ana", Kind::Camera);
        slots.assign("ana", Kind::Screen);
        slots.release("ana", Kind::Camera);
        assert_eq!(slots.find("ana", Kind::Screen), Some(1));
        assert_eq!(slots.assign("beto", Kind::Camera), Some(0));
    }
}
