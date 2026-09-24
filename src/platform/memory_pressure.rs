//! Pressão de memória do Android -> política de mídia do PR36.
//!
//! O `onTrimMemory` chega numa thread do Java, que não pode encostar em
//! `PapoApp` nem em `MediaStore`. Esta ponte só classifica e publica um
//! pedido num latch atômico; quem recolhe e aplica é a thread normal do Papo,
//! no quadro seguinte (ver `PapoApp::pump_memory_pressure`).
//!
//! A política em si não mora aqui: o Android vira um [`TrimLevel`] e pronto.
//! Manter a fronteira nesse ponto é o que permite uma futura origem Windows/
//! macOS/Linux alimentar a mesma política sem tocar no `MediaStore`.

use std::sync::atomic::{AtomicU8, Ordering};

use crate::media::TrimLevel;

// Constantes do `android.content.ComponentCallbacks2`. Os números crus ficam
// presos neste módulo: nem a Activity nem o MediaStore precisam deles.
//
// Do Android 14 em diante o sistema **parou de mandar** RUNNING_MODERATE,
// RUNNING_LOW e RUNNING_CRITICAL. Restaram UI_HIDDEN (troca de tela, app
// normal) e BACKGROUND e acima (processo no LRU, candidato a morrer). Os
// RUNNING_* continuam mapeados por causa dos aparelhos API 31-33, mas não são
// esperados no alvo atual: **não ter visto um RUNNING_CRITICAL não quer dizer
// que a ponte está quebrada.**
pub const TRIM_MEMORY_RUNNING_MODERATE: i32 = 5;
pub const TRIM_MEMORY_RUNNING_LOW: i32 = 10;
pub const TRIM_MEMORY_RUNNING_CRITICAL: i32 = 15;
pub const TRIM_MEMORY_UI_HIDDEN: i32 = 20;
pub const TRIM_MEMORY_BACKGROUND: i32 = 40;
// MODERATE (60) e COMPLETE (80) seguem a mesma direção de BACKGROUND.

/// Traduz o inteiro do Android para a política do PR36.
///
/// A escala não é monótona — `UI_HIDDEN` vale 20 e `RUNNING_CRITICAL`, que é
/// mais grave, vale 15 — então nem `match` por igualdade nem um limiar único
/// servem. Compara-se por faixa, na ordem de gravidade de cada era, e o que
/// sobra vira `Light` para não exagerar num valor intermediário desconhecido.
pub fn map_android_level(level: i32) -> TrimLevel {
    if level >= TRIM_MEMORY_BACKGROUND {
        // BACKGROUND/MODERATE/COMPLETE e qualquer coisa mais funda.
        TrimLevel::Critical
    } else if level >= TRIM_MEMORY_UI_HIDDEN {
        // A interface saiu de cena, mas o processo segue vivo e visível no
        // recents: é para ficar mais leve de manter, não para morrer.
        TrimLevel::Moderate
    } else if level >= TRIM_MEMORY_RUNNING_CRITICAL {
        TrimLevel::Critical
    } else if level >= TRIM_MEMORY_RUNNING_LOW {
        TrimLevel::Moderate
    } else {
        // RUNNING_MODERATE (5) e valores baixos/intermediários.
        TrimLevel::Light
    }
}

fn severity(level: TrimLevel) -> u8 {
    match level {
        TrimLevel::Light => 1,
        TrimLevel::Moderate => 2,
        TrimLevel::Critical => 3,
    }
}

fn from_severity(code: u8) -> Option<TrimLevel> {
    match code {
        1 => Some(TrimLevel::Light),
        2 => Some(TrimLevel::Moderate),
        3 => Some(TrimLevel::Critical),
        _ => None,
    }
}

/// Latch de um único inteiro de gravidade.
///
/// Pedidos que chegam antes de alguém recolher fundem-se no **mais grave**
/// (`fetch_max`): `Light` e depois `Critical` viram um `Critical`, em ordem
/// previsível, em vez de dois trims fora de ordem. Gravidade menor nunca
/// rebaixa um pedido maior pendente.
#[derive(Debug, Default)]
pub struct PendingTrim {
    severity: AtomicU8,
}

impl PendingTrim {
    pub const fn new() -> Self {
        Self {
            severity: AtomicU8::new(0),
        }
    }

    /// Publica um pedido. Pode ser chamado de qualquer thread.
    pub fn request(&self, level: TrimLevel) {
        self.severity.fetch_max(severity(level), Ordering::AcqRel);
    }

    /// Recolhe e zera. `None` quando não há nada pendente.
    pub fn take(&self) -> Option<TrimLevel> {
        match self.severity.swap(0, Ordering::AcqRel) {
            0 => None,
            code => from_severity(code),
        }
    }
}

/// O latch do processo. Sem persistência: se o Android matar o processo, tudo
/// o que estava em memória foi embora junto e o pedido pendente não importa.
static PENDING: PendingTrim = PendingTrim::new();

/// Publica um pedido no latch do processo.
pub fn request(level: TrimLevel) {
    PENDING.request(level);
}

/// Recolhe o pedido pendente, se houver.
pub fn take_pending() -> Option<TrimLevel> {
    PENDING.take()
}

/// Quem sabe aplicar a política de trim. Existe para o fanout ser testável
/// sem construir um `PapoApp` inteiro; a única implementação real é o
/// `MediaStore`.
pub trait TrimTarget {
    fn apply_trim(&mut self, level: TrimLevel);
}

impl TrimTarget for crate::media::MediaStore {
    fn apply_trim(&mut self, level: TrimLevel) {
        crate::media::MediaStore::trim(self, level);
    }
}

/// Aplica o nível a **todos** os `MediaStore` do processo — o da tela e o de
/// cada servidor guardado. Um servidor com cinco workspaces não pode escapar
/// do trim só porque não está visível.
pub fn trim_all<'a>(level: TrimLevel, stores: impl IntoIterator<Item = &'a mut dyn TrimTarget>) {
    let mut stores_applied = 0usize;
    for store in stores {
        store.apply_trim(level);
        stores_applied += 1;
    }
    log::debug!("memory trim fanout level={level:?} stores={stores_applied}");
}

/// Entrada JNI do `PapoActivity.onTrimMemory`. O trabalho aqui é o mínimo:
/// classificar, publicar, acordar a janela e voltar.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeTrimMemory(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    level: jni::sys::jint,
) {
    let mapped = map_android_level(level);
    PENDING.request(mapped);
    log::debug!("memory trim android_level={level} mapped={mapped:?} source=android");
    super::wake::request();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um alvo de falso que só anota o que recebeu, para provar que o fanout
    /// alcança cada store sem precisar de um MediaStore de verdade.
    #[derive(Default)]
    struct Recorder {
        levels: std::sync::Mutex<Vec<TrimLevel>>,
    }

    impl TrimTarget for Recorder {
        fn apply_trim(&mut self, level: TrimLevel) {
            self.levels.lock().unwrap().push(level);
        }
    }

    impl Recorder {
        fn got(&self) -> Vec<TrimLevel> {
            self.levels.lock().unwrap().clone()
        }
    }

    // (1) sem pendência não há nada a aplicar.
    #[test]
    fn sem_pendencia_devolve_none() {
        let latch = PendingTrim::new();
        assert_eq!(latch.take(), None);
    }

    // (2) um Light sai uma vez só; o latch zera ao ser recolhido.
    #[test]
    fn light_sai_uma_vez() {
        let latch = PendingTrim::new();
        latch.request(TrimLevel::Light);
        assert_eq!(latch.take(), Some(TrimLevel::Light));
        assert_eq!(latch.take(), None);
    }

    // (3) Moderate supera Light no mesmo intervalo de recolhimento.
    #[test]
    fn moderate_supera_light() {
        let latch = PendingTrim::new();
        latch.request(TrimLevel::Light);
        latch.request(TrimLevel::Moderate);
        assert_eq!(latch.take(), Some(TrimLevel::Moderate));
    }

    // (4) Critical supera Moderate.
    #[test]
    fn critical_supera_moderate() {
        let latch = PendingTrim::new();
        latch.request(TrimLevel::Moderate);
        latch.request(TrimLevel::Critical);
        assert_eq!(latch.take(), Some(TrimLevel::Critical));
    }

    // (5) gravidade menor não rebaixa um pedido maior pendente.
    #[test]
    fn menor_nao_rebaixa_maior() {
        let latch = PendingTrim::new();
        latch.request(TrimLevel::Critical);
        latch.request(TrimLevel::Light);
        latch.request(TrimLevel::Moderate);
        assert_eq!(latch.take(), Some(TrimLevel::Critical));
    }

    // (6) recolher limpa o latch.
    #[test]
    fn recolher_limpa() {
        let latch = PendingTrim::new();
        latch.request(TrimLevel::Moderate);
        assert!(latch.take().is_some());
        assert!(latch.take().is_none());
        latch.request(TrimLevel::Light);
        assert_eq!(latch.take(), Some(TrimLevel::Light));
    }

    // (7) pedidos concorrentes não perdem a maior gravidade.
    #[test]
    fn pedidos_concorrentes_guardam_a_maior() {
        let latch = std::sync::Arc::new(PendingTrim::new());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let levels = [
            TrimLevel::Light,
            TrimLevel::Moderate,
            TrimLevel::Critical,
            TrimLevel::Light,
        ];
        let mut handles = Vec::new();
        for level in levels {
            let latch = std::sync::Arc::clone(&latch);
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..1000 {
                    latch.request(level);
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(latch.take(), Some(TrimLevel::Critical));
    }

    // (8) UI_HIDDEN é a troca de tela normal: Moderate.
    #[test]
    fn ui_hidden_vira_moderate() {
        assert_eq!(
            map_android_level(TRIM_MEMORY_UI_HIDDEN),
            TrimLevel::Moderate
        );
    }

    // (9) BACKGROUND e mais fundo: Critical.
    #[test]
    fn background_vira_critical() {
        assert_eq!(
            map_android_level(TRIM_MEMORY_BACKGROUND),
            TrimLevel::Critical
        );
        assert_eq!(map_android_level(60), TrimLevel::Critical); // MODERATE
        assert_eq!(map_android_level(80), TrimLevel::Critical); // COMPLETE
    }

    // (10-12) legado RUNNING_* (API 31-33).
    #[test]
    fn running_legado_mapeia_por_gravidade() {
        assert_eq!(
            map_android_level(TRIM_MEMORY_RUNNING_MODERATE),
            TrimLevel::Light
        );
        assert_eq!(
            map_android_level(TRIM_MEMORY_RUNNING_LOW),
            TrimLevel::Moderate
        );
        assert_eq!(
            map_android_level(TRIM_MEMORY_RUNNING_CRITICAL),
            TrimLevel::Critical
        );
    }

    // (13) valores intermediários/desconhecidos caem por faixa, sem exagero.
    #[test]
    fn intermediarios_seguem_a_faixa() {
        assert_eq!(map_android_level(0), TrimLevel::Light);
        assert_eq!(map_android_level(7), TrimLevel::Light);
        assert_eq!(map_android_level(12), TrimLevel::Moderate);
        assert_eq!(map_android_level(18), TrimLevel::Critical);
        assert_eq!(map_android_level(30), TrimLevel::Moderate); // entre UI_HIDDEN e BACKGROUND
        assert_eq!(map_android_level(50), TrimLevel::Critical); // acima de BACKGROUND
    }

    // (14-16) o fanout alcança o ativo e **todos** os guardados; nenhum
    // workspace escapa.
    #[test]
    fn fanout_cobre_ativo_e_todos_os_guardados() {
        let mut active = Recorder::default();
        let mut stashed_a = Recorder::default();
        let mut stashed_b = Recorder::default();
        trim_all(
            TrimLevel::Moderate,
            [
                &mut active as &mut dyn TrimTarget,
                &mut stashed_a as &mut dyn TrimTarget,
                &mut stashed_b as &mut dyn TrimTarget,
            ],
        );
        assert_eq!(active.got(), vec![TrimLevel::Moderate]);
        assert_eq!(stashed_a.got(), vec![TrimLevel::Moderate]);
        assert_eq!(stashed_b.got(), vec![TrimLevel::Moderate]);
    }

    // Um segundo trim, mais grave, volta a alcançar todos — inclusive quem já
    // fora trimado.
    #[test]
    fn fanout_repetido_alcanca_todos_de_novo() {
        let mut active = Recorder::default();
        let mut stashed = Recorder::default();
        trim_all(
            TrimLevel::Light,
            [
                &mut active as &mut dyn TrimTarget,
                &mut stashed as &mut dyn TrimTarget,
            ],
        );
        trim_all(
            TrimLevel::Critical,
            [
                &mut active as &mut dyn TrimTarget,
                &mut stashed as &mut dyn TrimTarget,
            ],
        );
        assert_eq!(active.got(), vec![TrimLevel::Light, TrimLevel::Critical]);
        assert_eq!(stashed.got(), vec![TrimLevel::Light, TrimLevel::Critical]);
    }
}
