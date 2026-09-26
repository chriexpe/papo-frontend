//! Permissões do Android, pedidas na hora em que fazem falta.
//!
//! No Android o manifesto só declara o que o aplicativo *pode* pedir; quem
//! dá é o usuário, numa caixa de diálogo, e só quando alguém pede. Pedir na
//! abertura, sem nada à vista que explique o porquê, é o jeito mais rápido
//! de ouvir não — então quem pede é o próprio botão de gravar, no momento em
//! que é tocado.
//!
//! A resposta chega pela Activity, numa thread do Java, e por isso o estado
//! mora aqui num mapa em vez de num valor de retorno.

use std::collections::HashMap;
use std::sync::Mutex;

/// Gravar áudio do microfone.
pub const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";
/// Abrir a câmera.
pub const CAMERA: &str = "android.permission.CAMERA";

/// Em que pé está uma permissão.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Pode usar.
    Granted,
    /// A caixa está na tela, ou a resposta ainda não voltou.
    Asking,
    /// O usuário disse não.
    Denied,
}

static STATE: Mutex<Option<HashMap<String, Status>>> = Mutex::new(None);

/// Garante a permissão, pedindo-a se for a primeira vez.
///
/// Nunca bloqueia: devolve [`Status::Asking`] enquanto a caixa está na tela.
/// Quem chama tenta de novo no próximo toque — um toque a mais na primeira
/// vez, e nenhum depois.
pub fn ensure(permission: &str) -> Status {
    let Ok(mut guard) = STATE.lock() else {
        return Status::Denied;
    };
    let known = guard.get_or_insert_with(HashMap::new);
    if let Some(status) = known.get(permission) {
        return *status;
    }
    drop(guard);

    // O Android lembra a resposta entre execuções; este mapa, não. Perguntar
    // antes de pedir é o que evita gastar o primeiro toque depois de cada
    // abertura com uma caixa que nem vai aparecer.
    if super::jvm::call_activity_bool("hasPermission", "(Ljava/lang/String;)Z", Some(permission))
        == Some(true)
    {
        remember(permission, Status::Granted);
        return Status::Granted;
    }

    remember(permission, Status::Asking);
    log::info!("pedindo permissão: {permission}");
    let asked = super::jvm::call_activity(
        "requestPermission",
        "(Ljava/lang/String;)V",
        Some(permission),
    );
    if !asked {
        // Sem conseguir pedir, não adianta ficar esperando resposta.
        remember(permission, Status::Denied);
        return Status::Denied;
    }
    Status::Asking
}

fn remember(permission: &str, status: Status) {
    if let Ok(mut guard) = STATE.lock() {
        guard
            .get_or_insert_with(HashMap::new)
            .insert(permission.to_owned(), status);
    }
}

/// A Activity respondeu.
///
/// O nome é o que o JNI exige: `Java_` + pacote e classe com `_` no lugar
/// dos pontos + o nome do método.
///
/// # Safety
/// Chamada pelo JNI.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativePermissionResult(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    permission: jni::objects::JString,
    granted: bool,
) {
    let Ok(name) = env.get_string(&permission) else {
        return;
    };
    let name: String = name.into();
    let status = if granted {
        Status::Granted
    } else {
        Status::Denied
    };
    log::info!("permissão {name}: {status:?}");

    if let Ok(mut guard) = STATE.lock() {
        guard
            .get_or_insert_with(HashMap::new)
            .insert(name, status);
    }
    // A resposta chegou de uma thread do Java: sem pedir quadro, a
    // interface só perceberia no próximo toque.
    super::wake::request();
}
