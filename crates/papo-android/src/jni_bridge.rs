use super::*;
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JObject, JString, JValue},
    sys::{jlong, jstring},
};
use papo_core::storage::{Secret, StorageError, StorageResult};
use std::collections::HashMap;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicI64, Ordering},
};

enum Backend {
    Live(Box<Client>),
    Demo(Box<demo::Demo>),
}
impl Backend {
    fn poll(&mut self) -> Value {
        match self {
            Self::Live(c) => c.poll(),
            Self::Demo(c) => c.poll(),
        }
    }
    fn dispatch(&mut self, action: Action) -> Result<(), String> {
        match self {
            Self::Live(c) => c.dispatch(action),
            Self::Demo(c) => c.dispatch(action),
        }
    }
}
static CLIENTS: OnceLock<Mutex<HashMap<i64, Backend>>> = OnceLock::new();
static NEXT: AtomicI64 = AtomicI64::new(1);
fn clients() -> &'static Mutex<HashMap<i64, Backend>> {
    CLIENTS.get_or_init(Default::default)
}

struct AndroidSecrets {
    vm: Arc<JavaVM>,
    object: GlobalRef,
}
impl AndroidSecrets {
    fn call(
        &self,
        method: &str,
        signature: &str,
        server: &str,
        secret: Secret,
        value: Option<&str>,
    ) -> StorageResult<Option<String>> {
        let mut env = self
            .vm
            .attach_current_thread()
            .map_err(|e| StorageError::new(e.to_string()))?;
        // Network workers live for hours. A local frame bounds JNI string refs
        // even when this thread was already attached before entering here.
        let result = env.with_local_frame(8, |env| -> jni::errors::Result<Option<String>> {
            let server = env.new_string(server)?;
            let key = env.new_string(secret.key())?;
            let mut args = vec![
                JValue::Object(server.as_ref()),
                JValue::Object(key.as_ref()),
            ];
            let value = value.map(|v| env.new_string(v)).transpose()?;
            if let Some(v) = &value {
                args.push(JValue::Object(v.as_ref()));
            }
            let returned = env.call_method(self.object.as_obj(), method, signature, &args)?;
            if method == "load" {
                let object = returned.l()?;
                if object.is_null() {
                    Ok(None)
                } else {
                    Ok(Some(env.get_string(&JString::from(object))?.into()))
                }
            } else {
                Ok(None)
            }
        });
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err(StorageError::new("Android Keystore operation failed"));
        }
        result.map_err(|e| StorageError::new(e.to_string()))
    }
}
impl SecretStore for AndroidSecrets {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        self.call(
            "load",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            server,
            secret,
            None,
        )
    }
    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        self.call(
            "store",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            server,
            secret,
            Some(value),
        )
        .map(|_| ())
    }
    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()> {
        self.call(
            "remove",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            server,
            secret,
            None,
        )
        .map(|_| ())
    }
}

fn guarded<T: Default>(env: &mut JNIEnv, f: impl FnOnce(&mut JNIEnv) -> Result<T, String>) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(env))) {
        Ok(Ok(value)) => value,
        result => {
            let error = match result {
                Ok(Err(e)) => e,
                _ => "The native core encountered an unexpected error".into(),
            };
            if !env.exception_check().unwrap_or(false) {
                let _ = env.throw_new("java/lang/IllegalStateException", error);
            }
            T::default()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_compose_NativeCore_open(
    mut env: JNIEnv,
    _this: JObject,
    url: JString,
    path: JString,
    secrets: JObject,
    listener: JObject,
) -> jlong {
    guarded(&mut env, |env| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("PapoCompose")
                .with_max_level(log::LevelFilter::Info),
        );
        let url: String = env.get_string(&url).map_err(|e| e.to_string())?.into();
        let path: String = env.get_string(&path).map_err(|e| e.to_string())?.into();
        let vm = Arc::new(env.get_java_vm().map_err(|e| e.to_string())?);
        let storage = Arc::new(AndroidSecrets {
            vm: Arc::clone(&vm),
            object: env.new_global_ref(secrets).map_err(|e| e.to_string())?,
        });
        let listener = env.new_global_ref(listener).map_err(|e| e.to_string())?;
        let wake = Wake::new(move || {
            if let Ok(mut env) = vm.attach_current_thread() {
                let _ = env.call_method(listener.as_obj(), "onWake", "()V", &[]);
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
            }
        });
        let client = Client::open(&url, PathBuf::from(path), storage, wake)?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        clients()
            .lock()
            .map_err(|_| "Native client lock failed")?
            .insert(id, Backend::Live(Box::new(client)));
        Ok(id)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_compose_NativeCore_openDemo(
    mut env: JNIEnv,
    _this: JObject,
) -> jlong {
    guarded(&mut env, |_| {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        clients()
            .lock()
            .map_err(|_| "Native client lock failed")?
            .insert(id, Backend::Demo(Box::default()));
        Ok(id)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_compose_NativeCore_poll(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jstring {
    guarded(&mut env, |env| {
        let value = clients()
            .lock()
            .map_err(|_| "Native client lock failed")?
            .get_mut(&handle)
            .ok_or("Client is closed")?
            .poll();
        Ok(env
            .new_string(value.to_string())
            .map_err(|e| e.to_string())?
            .into_raw())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_compose_NativeCore_dispatch(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    action: JString,
) {
    guarded(&mut env, |env| {
        let action: String = env.get_string(&action).map_err(|e| e.to_string())?.into();
        let action =
            serde_json::from_str(&action).map_err(|e| format!("Invalid native command: {e}"))?;
        clients()
            .lock()
            .map_err(|_| "Native client lock failed")?
            .get_mut(&handle)
            .ok_or("Client is closed")?
            .dispatch(action)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_compose_NativeCore_close(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    guarded(&mut env, |_| {
        let removed = clients()
            .lock()
            .map_err(|_| "Native client lock failed")?
            .remove(&handle);
        drop(removed);
        Ok(())
    })
}
