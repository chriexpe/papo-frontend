//! Chamar a Activity a partir do Rust.
//!
//! Até aqui o tráfego era de mão única: a Activity empurrava bordas, texto
//! do teclado. Permissão e seletor de arquivos invertem o sentido — quem
//! começa é o Rust, quando alguém toca em gravar ou em anexar — e para isso
//! é preciso falar JNI com a Activity de verdade.
//!
//! O `android-activity` guarda os dois ponteiros de que isso depende: a
//! máquina virtual e o objeto da Activity. O resto é ligar a thread atual à
//! VM (a do desenho não é uma thread Java) e chamar o método.

use android_activity::AndroidApp;

use std::sync::OnceLock;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Guarda a Activity. Chamado uma vez, no `android_main`.
pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Chama um método sem retorno da `PapoActivity`, com um texto de
/// argumento (ou nenhum).
///
/// `signature` é a assinatura JNI do método — `"()V"` para um sem
/// argumentos, `"(Ljava/lang/String;)V"` para um que recebe texto. Errar
/// aqui não dá erro de compilação: dá método não encontrado em execução.
///
/// O texto é criado dentro da chamada porque só aqui existe o `JNIEnv` que
/// sabe fabricá-lo.
pub fn call_activity(method: &str, signature: &str, text: Option<&str>) -> bool {
    call(method, signature, text).is_some()
}

/// Chama um método da Activity que devolve `boolean`.
pub fn call_activity_bool(method: &str, signature: &str, text: Option<&str>) -> Option<bool> {
    call(method, signature, text)
}

/// `None` quer dizer que a chamada não aconteceu. Para um método sem
/// retorno, o `false` de dentro não significa nada — o que importa é ter
/// voltado `Some`.
fn call(method: &str, signature: &str, text: Option<&str>) -> Option<bool> {
    let Some(app) = APP.get() else {
        log::error!("sem Activity para chamar {method}");
        return None;
    };
    let vm = match unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) } {
        Ok(vm) => vm,
        Err(error) => {
            log::error!("sem máquina virtual: {error}");
            return None;
        }
    };
    // A thread que desenha não é uma thread Java: ligá-la à VM é o que dá
    // direito de chamar qualquer coisa daqui.
    let mut env = match vm.attach_current_thread() {
        Ok(env) => env,
        Err(error) => {
            log::error!("não deu para entrar na máquina virtual: {error}");
            return None;
        }
    };
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };

    let outcome = match text {
        Some(text) => match env.new_string(text) {
            Ok(value) => {
                let args = [jni::objects::JValue::Object(&value)];
                env.call_method(&activity, method, signature, &args)
            }
            Err(error) => {
                log::error!("{method}: texto não virou String do Java: {error}");
                return None;
            }
        },
        None => env.call_method(&activity, method, signature, &[]),
    };

    match outcome {
        Ok(value) => Some(value.z().unwrap_or(false)),
        Err(error) => {
            // Uma exceção pendente trava qualquer chamada seguinte; limpar
            // é o que mantém a ponte utilizável depois de um erro.
            let _ = env.exception_clear();
            log::error!("{method} falhou: {error}");
            None
        }
    }
}
