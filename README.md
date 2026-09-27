# Papo

Cliente nativo e multiplataforma para o [Papo](https://github.com/chriexpe/papo-backend), escrito em Rust com [egui](https://github.com/emilk/egui)/eframe.

O Papo conecta a múltiplos servidores ao mesmo tempo, mantém estado em tempo real por WebSocket, usa WebRTC/GStreamer para chamadas e mídia e integra cada plataforma sem Electron. Conteúdo web incorporado usa o motor nativo disponível em cada sistema.

> **Estado atual:** para a superfície hoje exposta pelo backend, o cliente está praticamente feature-complete. Os dois maiores recursos ainda pendentes no frontend são **rich presence** e **captura/publicação de compartilhamento de tela**.

## Recursos

- **Chat completo:** respostas, edição, exclusão, fixados, reações, menções, histórico paginado, busca avançada e rascunhos por canal.
- **Multi-servidor:** sessões independentes, reconexão, reconciliação após períodos offline, cache persistente e fila segura para mensagens pendentes.
- **Mídia:** imagens, arquivos, vídeo e áudio inline, waveform, mensagens de voz, visualizador de imagens e downloads sob demanda.
- **Links ricos:** Open Graph, oEmbed, mídia direta e players incorporados; links externos passam pelo fluxo de confiança do cliente.
- **Chamadas:** voz e vídeo por WebRTC, mute, câmera, indicador de fala e UI compacta/flutuante sem interromper a chamada ao navegar pelo app.
- **Perfis e presença:** avatar, banner, bio, status, presença, cargos, cartão de perfil e editor de recorte de imagens.
- **Administração:** canais, cargos, permissões globais e por canal, membros, servidor, figurinhas e auditoria.
- **Integração nativa:** notificações, bandeja, inicialização com o sistema, tema do sistema, seletor de arquivos e atualizações onde a plataforma permite.
- **Idiomas:** português do Brasil e inglês.

## Plataformas

| Plataforma | Arquitetura | Distribuição | Integrações principais |
| --- | --- | --- | --- |
| Linux | x86_64, aarch64 | Flatpak, .deb, tar.gz | WPE WebKit, GStreamer, tray/notificações e integrações desktop |
| Windows | x86_64 | Setup.exe, ZIP portátil | WebView2, GStreamer privado, tray, notificações, autostart e updater |
| Android | arm64-v8a | APK assinado | mesma UI egui, Android WebView, mídia/chamadas e updater para sideload |

Windows e Android podem consumir novas versões publicadas no GitHub Releases pelo fluxo de atualização do próprio app. macOS e web ainda não são alvos de release suportados.

## Instalação

Os pacotes prontos ficam em [GitHub Releases](https://github.com/chriexpe/papo-frontend/releases).

No Linux, para rodar a partir do código-fonte:

```bash
git clone https://github.com/chriexpe/papo-frontend.git
cd papo-frontend
./scripts/deps.sh --install
cargo run
```

No Android:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
scripts/android.sh --run
```

O projeto requer **Rust 1.98+**. Para validar o workspace:

```bash
cargo test --locked --workspace
cargo clippy --locked --all-targets -- -D warnings
```

## Arquitetura

A maior parte da lógica compartilhada vive em `crates/papo-core`: API REST/WebSocket, sessões, sincronização, cache e runtime de rede. A interface fica em Rust/egui, enquanto mídia, chamadas, WebEmbed e integrações do sistema são separadas por backend de plataforma.

Essa divisão mantém o comportamento do cliente consistente entre desktop e Android sem transformar cada plataforma em uma implementação diferente do Papo.

## Falta fazer

- **Rich presence:** integrar atividades externas como jogos, música e ferramentas de desenvolvimento ao perfil/presença.
- **Compartilhamento de tela:** adicionar captura nativa por plataforma e publicação da stream na chamada.

Recursos que dependem de novos contratos do backend entram conforme essa superfície for adicionada.

## Licença

AGPL-3.0-or-later.
