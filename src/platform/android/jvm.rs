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
    call(method, signature, text, |_| ()).is_some()
}

/// Chama um método da Activity que recebe um `boolean` e não devolve nada.
pub fn call_activity_flag(method: &str, flag: bool) -> bool {
    let Some(app) = APP.get() else {
        log::error!("sem Activity para chamar {method}");
        return false;
    };
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return false;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return false;
    };
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let args = [jni::objects::JValue::Bool(flag.into())];
    match env.call_method(&activity, method, "(Z)V", &args) {
        Ok(_) => true,
        Err(error) => {
            let _ = env.exception_clear();
            log::error!("{method} falhou: {error}");
            false
        }
    }
}

/// Chama um método da Activity que devolve `boolean`.
pub fn call_activity_bool(method: &str, signature: &str, text: Option<&str>) -> Option<bool> {
    call(method, signature, text, |value| value.z().unwrap_or(false))
}

/// Chama um método da Activity sem argumentos que devolve `int`.
pub fn call_activity_int(method: &str) -> Option<i32> {
    call(method, "()I", None, |value| value.i().ok()).flatten()
}
/// Chama um método da Activity sem argumentos que devolve `String`.
pub fn call_activity_string(method: &str) -> Option<String> {
    let Some(app) = APP.get() else {
        log::error!("sem Activity para chamar {method}");
        return None;
    };
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let value = env
        .call_method(&activity, method, "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    if value.is_null() {
        return None;
    }
    let value = jni::objects::JString::from(value);
    env.get_string(&value).ok().map(Into::into)
}


/// Chama um método da Activity que recebe texto + inteiro e devolve `byte[]`.
///
/// Usado por renderizações pequenas que a própria plataforma sabe fazer
/// melhor que Rust (por exemplo emoji colorido com o stack tipográfico do
/// Android). O array é copiado antes de soltar o `JNIEnv`.
pub fn call_activity_bytes_with_text_int(
    method: &str,
    text: &str,
    value: i32,
) -> Option<Vec<u8>> {
    let app = APP.get()?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let text = env.new_string(text).ok()?;
    let args = [
        jni::objects::JValue::Object(&text),
        jni::objects::JValue::Int(value),
    ];
    let returned = match env.call_method(
        &activity,
        method,
        "(Ljava/lang/String;I)[B",
        &args,
    ) {
        Ok(result) => result.l().ok()?,
        Err(error) => {
            let _ = env.exception_clear();
            log::error!("{method} falhou: {error}");
            return None;
        }
    };
    if returned.is_null() {
        return None;
    }
    let bytes = jni::objects::JByteArray::from(returned);
    env.convert_byte_array(&bytes).ok()
}

/// `None` quer dizer que a chamada não aconteceu. `read` tira do retorno o
/// tipo que o método promete; ele roda enquanto o `JNIEnv` ainda existe.
fn call<T>(
    method: &str,
    signature: &str,
    text: Option<&str>,
    read: impl FnOnce(jni::objects::JValueOwned<'_>) -> T,
) -> Option<T> {
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
        Ok(value) => Some(read(value)),
        Err(error) => {
            // Uma exceção pendente trava qualquer chamada seguinte; limpar
            // é o que mantém a ponte utilizável depois de um erro.
            let _ = env.exception_clear();
            log::error!("{method} falhou: {error}");
            None
        }
    }
}
