# Papo

Cliente nativo e multiplataforma para o [Papo](https://github.com/Papo-Chat/papo-backend), escrito em Rust com [egui](https://github.com/emilk/egui)/eframe.

O Papo foi feito para manter a mesma experiência entre desktop e Android sem Electron, com suporte simultâneo a múltiplos servidores e integrações nativas de cada plataforma.

> **Estado atual:** o cliente está próximo de feature-complete para o que o backend oferece hoje. Os principais recursos ainda pendentes são **rich presence** e **compartilhamento de tela**.

## Destaques

- múltiplos servidores conectados ao mesmo tempo, com cache local, reconexão e sincronização após períodos offline;
- voz e vídeo por WebRTC/GStreamer;
- mídia e previews ricos, incluindo players web nativos por plataforma;
- interface e integrações nativas em Linux, Windows e Android.

## Plataformas

| Plataforma | Arquitetura | Distribuição |
| --- | --- | --- |
| Linux | x86_64, aarch64 | Flatpak, .deb, tar.gz |
| Windows | x86_64 | Setup.exe, ZIP portátil |
| macOS (experimental) | Apple Silicon (aarch64) | Self-contained ad-hoc signed CI `.app` |
| Android | arm64-v8a | APK assinado |

No Windows, o Papo usa WebView2 e distribui seu próprio runtime do GStreamer. No Android, a mesma aplicação Rust/egui é empacotada como app nativo. Linux usa WPE WebKit para conteúdo web incorporado.

Windows e instalações Android via sideload têm atualização pelo próprio app usando GitHub Releases.

macOS tem um `.app` experimental (assinatura ad-hoc, sem notarização) no CI para Apple Silicon, com GStreamer incluído no pacote e sem integrações completas de sistema; veja [docs/macos.md](docs/macos.md). A versão web também não é suportada.

## Instalação

Os pacotes prontos ficam em [GitHub Releases](https://github.com/chriexpe/papo-frontend/releases).

Para rodar no Linux a partir do código-fonte:

```bash
git clone https://github.com/chriexpe/papo-frontend.git
cd papo-frontend
./scripts/deps.sh --install
cargo install patch-crate --version 0.1.13 --locked
python3 scripts/prepare-winit.py
cargo run
```

Para desenvolvimento no Android:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
cargo install patch-crate --version 0.1.13 --locked
scripts/android.sh --run
```

Notificações por push (FCM) precisam da configuração do app Android no projeto Firebase, os campos do `google-services.json`. Ela não fica no repositório: vem do ambiente ou de `~/.gradle/gradle.properties`.

```properties
PAPO_FIREBASE_APP_ID=1:…:android:…      # client[].client_info.mobilesdk_app_id
PAPO_FIREBASE_API_KEY=AIza…             # client[].api_key[].current_key
PAPO_FIREBASE_PROJECT_ID=papo           # project_info.project_id
PAPO_FIREBASE_SENDER_ID=…               # project_info.project_number
```

Sem ela o APK sai sem FCM, e as notificações com o app fechado ficam com a verificação periódica do WorkManager.

O projeto requer **Rust 1.98+**.

A correção de visibilidade da janela no Wayland fica em um arquivo `.patch`.
O preparo usa **Python 3.11+**, **Git** e `patch-crate 0.1.13`; repita
`python3 scripts/prepare-winit.py` após limpar o `target` ou alterar o patch.
No Windows, use `python` no lugar de `python3`. O procedimento para atualizar
ou remover a correção está em [docs/winit-patch.md](docs/winit-patch.md).

```bash
cargo test --locked --workspace
cargo clippy --locked --all-targets -- -D warnings
```

## Arquitetura

A lógica compartilhada de API, sessões, sincronização, cache e runtime de rede vive em `crates/papo-core`. A UI fica em Rust/egui; mídia, chamadas, WebEmbed e integrações do sistema têm backends específicos por plataforma.

## Em andamento

- **Rich presence**
- **Compartilhamento de tela**

## Licença

AGPL-3.0-or-later.
