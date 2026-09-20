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

    /// Acerta os lugares com quem se quer ver: larga quem saiu da lista,
    /// senta quem cabe, e devolve as duas listas para quem chama avisar o
    /// servidor. O que não couber continua em `wanted` e senta sozinho na
    /// próxima vez que um lugar vagar.
    pub fn reconcile(&mut self, wanted: &[String], kind: Kind) -> (Vec<String>, Vec<String>) {
        let mut leaving = Vec::new();
        for place in &mut self.places {
            let Some(slot) = place.as_ref() else { continue };
            if slot.kind == kind && !wanted.contains(&slot.publisher) {
                leaving.push(slot.publisher.clone());
                *place = None;
            }
        }

        let mut entering = Vec::new();
        for publisher in wanted {
            if self.find(publisher, kind).is_some() {
                continue;
            }
            if self.assign(publisher, kind).is_some() {
                entering.push(publisher.clone());
            }
        }
        (leaving, entering)
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

    /// O sétimo vídeo não se perde: ele fica esperando, e senta no lugar
    /// que a primeira câmera a desligar deixar. Contar isto do lado de fora
    /// era o furo — lá não se sabe o que coube.
    #[test]
    fn quem_nao_coube_senta_quando_um_lugar_vaga() {
        let mut slots = Slots::new(2);
        let three = ["ana".to_owned(), "beto".to_owned(), "clara".to_owned()];
        let (leaving, entering) = slots.reconcile(&three, Kind::Camera);
        assert!(leaving.is_empty());
        assert_eq!(entering, ["ana", "beto"]);
        assert!(slots.find("clara", Kind::Camera).is_none());

        // A Ana desligou a câmera: sai da lista, e a Clara ocupa o lugar.
        let two = ["beto".to_owned(), "clara".to_owned()];
        let (leaving, entering) = slots.reconcile(&two, Kind::Camera);
        assert_eq!(leaving, ["ana"]);
        assert_eq!(entering, ["clara"]);
        assert!(slots.find("clara", Kind::Camera).is_some());
    }

    /// Pedir de novo a mesma lista não fala com o servidor.
    #[test]
    fn lista_repetida_nao_pede_nada() {
        let mut slots = Slots::new(3);
        let wanted = ["ana".to_owned()];
        slots.reconcile(&wanted, Kind::Camera);
        let (leaving, entering) = slots.reconcile(&wanted, Kind::Camera);
        assert!(leaving.is_empty() && entering.is_empty());
    }

    /// Largar a câmera de alguém não mexe na tela que a mesma pessoa
    /// compartilha: são duas tracks, dois lugares.
    #[test]
    fn desligar_a_camera_libera_so_a_camera() {
        let mut slots = Slots::new(2);
        slots.assign("ana", Kind::Camera);
        slots.assign("ana", Kind::Screen);
        slots.reconcile(&[], Kind::Camera);
        assert_eq!(slots.find("ana", Kind::Screen), Some(1));
        assert_eq!(slots.assign("beto", Kind::Camera), Some(0));
    }
}
