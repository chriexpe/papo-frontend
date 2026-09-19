//! Restauração da janela pelo KWin.
//!
//! O Wayland não deixa um aplicativo se desminimizar: `set_visible` e
//! `set_minimized(false)` não existem, e o xdg-activation só faz o KWin
//! piscar a barra de tarefas. O caminho que resta é a API de scripts do
//! próprio KWin: carrega um script curto, executa e descarrega — a mesma
//! técnica do kdotool.

use std::path::PathBuf;

const PLUGIN: &str = "papo-restore";

/// Desminimiza e ativa a janela com este `app_id`, sem bloquear quem chamou.
pub fn restore_window(app_id: &str) {
    let app_id = app_id.to_owned();
    std::thread::Builder::new()
        .name("papo-kwin".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            if let Err(error) = runtime.block_on(run_script(&app_id)) {
                log::debug!("kwin não restaurou a janela: {error}");
            }
        })
        .ok();
}

async fn run_script(app_id: &str) -> zbus::Result<()> {
    let path = write_script(app_id)
        .ok_or_else(|| zbus::Error::Failure("não consegui escrever o script".into()))?;
    let connection = zbus::Connection::session().await?;

    // Um plugin com o mesmo nome pode ter ficado de uma tentativa anterior.
    let _ = call(&connection, "/Scripting", "unloadScript", &(PLUGIN,)).await;

    let id: i32 = call(
        &connection,
        "/Scripting",
        "loadScript",
        &(path.to_string_lossy().to_string(), PLUGIN),
    )
    .await?
    .body()
    .deserialize()?;

    // `start()` não roda scripts carregados em tempo de execução; `run()` sim.
    let _ = connection
        .call_method(
            Some("org.kde.KWin"),
            format!("/Scripting/Script{id}").as_str(),
            Some("org.kde.kwin.Script"),
            "run",
            &(),
        )
        .await?;

    let _ = call(&connection, "/Scripting", "unloadScript", &(PLUGIN,)).await;
    Ok(())
}

async fn call<B: serde::Serialize + zvariant::DynamicType>(
    connection: &zbus::Connection,
    path: &str,
    method: &str,
    body: &B,
) -> zbus::Result<zbus::Message> {
    connection
        .call_method(
            Some("org.kde.KWin"),
            path,
            Some("org.kde.kwin.Scripting"),
            method,
            body,
        )
        .await
        .map(|reply| reply.clone())
}

fn write_script(app_id: &str) -> Option<PathBuf> {
    let script = format!(
        r#"// gerado pelo Papo
const wanted = "{app_id}";
for (const w of workspace.windowList()) {{
    const cls = String(w.resourceClass || "").toLowerCase();
    const name = String(w.resourceName || "").toLowerCase();
    if (cls === wanted || name === wanted) {{
        w.minimized = false;
        workspace.activeWindow = w;
    }}
}}
"#
    );
    let path = std::env::temp_dir().join("papo-restore.js");
    std::fs::write(&path, script).ok()?;
    Some(path)
}
