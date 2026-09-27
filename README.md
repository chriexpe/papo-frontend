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
| Android | arm64-v8a | APK assinado |

No Windows, o Papo usa WebView2 e distribui seu próprio runtime do GStreamer. No Android, a mesma aplicação Rust/egui é empacotada como app nativo. Linux usa WPE WebKit para conteúdo web incorporado.

Windows e instalações Android via sideload têm atualização pelo próprio app usando GitHub Releases.

macOS e web ainda não são alvos de release suportados.

## Instalação

Os pacotes prontos ficam em [GitHub Releases](https://github.com/chriexpe/papo-frontend/releases).

Para rodar no Linux a partir do código-fonte:

```bash
git clone https://github.com/chriexpe/papo-frontend.git
cd papo-frontend
./scripts/deps.sh --install
cargo run
```

Para desenvolvimento no Android:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
scripts/android.sh --run
```

O projeto requer **Rust 1.98+**.

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
