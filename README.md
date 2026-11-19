# Papo

Clientes nativos para o [Papo](https://github.com/Papo-Chat/papo-backend), sem Electron: Android em Kotlin/Jetpack Compose e desktop em Rust com [Rune](https://github.com/anomalyco/rune), os dois sobre um SDK Rust comum.

> **Estado atual:** o cliente desktop está sendo reconstruído do zero sobre o SDK e ainda não tem versão publicada. O cliente egui anterior foi aposentado; o último código dele está na etiqueta `egui-final`. O Android continua publicado.

## Plataformas

| Plataforma | Arquitetura | Distribuição |
| --- | --- | --- |
| Android | arm64-v8a | APK assinado em [GitHub Releases](https://github.com/chriexpe/papo-frontend/releases) |
| Linux | — | em desenvolvimento |
| Windows, macOS | — | depois do Linux |

## Arquitetura

| Caminho | O quê |
| --- | --- |
| `crates/sdk/` | SDK do cliente: `api` (REST e gateway), `core` (contas, sincronização, estado, cache), `ui` (listas e timelines para as interfaces), `ffi` (UniFFI), `test-server` |
| `crates/papo-desktop/` | Cliente desktop |
| `crates/papo-call/` | Chamadas WebRTC/GStreamer |
| `crates/papo-core/`, `crates/papo-android/` | Núcleo antigo e sua ponte UniFFI, usados pelo Android até ele migrar para o SDK |
| `android/compose/` | Cliente Android |

## Desenvolvimento

O projeto requer **Rust 1.99+**. As dependências de sistema (GStreamer e afins):

```bash
./scripts/deps.sh --install
cargo test --locked --workspace --exclude papo-desktop
cargo clippy --locked --workspace --exclude papo-desktop --all-targets -- -D warnings
```

Android:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk --locked
scripts/android-compose.sh --arm64 --run
```

Notificações por push (FCM) precisam da configuração do app Android no projeto Firebase, os campos do `google-services.json`. Ela não fica no repositório: vem do ambiente ou de `~/.gradle/gradle.properties`.

```properties
PAPO_FIREBASE_APP_ID=1:…:android:…      # client[].client_info.mobilesdk_app_id
PAPO_FIREBASE_API_KEY=AIza…             # client[].api_key[].current_key
PAPO_FIREBASE_PROJECT_ID=papo           # project_info.project_id
PAPO_FIREBASE_SENDER_ID=…               # project_info.project_number
```

Sem ela o APK sai sem FCM, e as notificações com o app fechado ficam com a verificação periódica do WorkManager.

## Licença

AGPL-3.0-or-later.
