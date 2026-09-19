# Papo — cliente nativo

Interface do [Papo](https://github.com/Papo-Chat/papo-backend) escrita em Rust com
[egui](https://github.com/emilk/egui)/eframe. Fala com o backend Go por REST e WebSocket,
sem navegador embutido: um binário só, sem Electron, sem webview.

## Rodando

```bash
cargo run                    # abre a janela
./scripts/watch.sh           # recompila e reabre a cada mudança em src/ ou assets/
./scripts/install-desktop.sh # instala papo.desktop e os ícones no ~/.local/share
cargo test                   # testes
```

Ferramentas de conferência, sem interface:

```bash
cargo run -- demo                         # abre a janela com um servidor de mentira
cargo run -- selftest <usuário> <senha>   # exercita REST: login, whoami, canais, pessoas
cargo run -- media-test                   # gera vídeo e áudio e os toca pelo player
cargo run -- pick-test                    # abre o seletor de arquivos do sistema
cargo run -- notify-test                  # dispara uma notificação de exemplo
PAPO_SERVER=http://localhost:8080 cargo run -- selftest
```

O `demo` é o modo de trabalho para a interface: povoa canais, pessoas e mensagens sem
rede e **gera a mídia de verdade** (imagem, vídeo Theora e áudio Ogg) direto no cache de
anexos, então imagem, vídeo, áudio, reações e figurinhas passam pelo mesmo caminho que
passariam vindos do servidor.

O endereço do servidor também é editável na tela de entrada e fica guardado nos ajustes.

## Como está organizado

```
src/
  api/        cliente REST, WebSocket e a ponte com a interface
    client.rs   reqwest + cookie de sessão (Auth), persistido em disco
    ws.rs       socket com heartbeat e reconexão progressiva
    net.rs      thread com runtime tokio; a janela troca comandos e updates por canais
  state/      estado da aplicação, alimentado pelas respostas e pelos eventos
    demo.rs     servidor de mentira para trabalhar na interface sem backend
  media/      anexos: download, cache, texturas e reprodução
    mod.rs      fila de download, cache em disco e as texturas da interface
    player.rs   GStreamer: vídeo em textura, áudio, forma de onda e gravação
  ui/         telas e sistema de design
    theme.rs    tokens (cor, tipografia, espaço) e o estilo do egui
    glass.rs    desfoque de fundo em OpenGL — o vidro fosco
    shell.rs    janela principal: canais · conversa · membros
    attachments.rs  imagem, vídeo, áudio e arquivo dentro da mensagem
    viewer.rs   tela cheia com zoom, arraste e navegação entre anexos
    emoji.rs    seletor, reações e o texto que mistura emoji com palavras
    emoji_raster.rs  emoji colorido tirado da fonte do sistema, via swash
    auth.rs     entrada e primeiro uso da instância
  platform/   integração com a área de trabalho (só Linux por enquanto)
    global_menu.rs  serviço com.canonical.dbusmenu
    appmenu.rs      liga o menu à wl_surface (protocolo do Plasma)
    tray.rs         ícone na bandeja (StatusNotifierItem, via ksni)
    notify.rs       notificações (org.freedesktop.Notifications)
    kwin.rs         restaura a janela minimizada pela API de scripts do KWin
    activate.rs     xdg-activation, para levantar a janela
    blur.rs         desfoque do compositor (ver a ressalva abaixo)
    desktop.rs      cor de destaque, tema claro/escuro e fonte, lidos do sistema
    launcher.rs     contador na barra de tarefas (com.canonical.Unity.LauncherEntry)
    files.rs        seletor de arquivos e pasta de downloads, pelo xdg-desktop-portal
```

A interface é imediata: nenhuma chamada de rede acontece dentro de um quadro. Tudo passa
por canais, e quem responde acorda a janela com `request_repaint()` — sem isso a resposta
ficaria parada até o próximo movimento do mouse.

## Desenho

Linguagem visual seguindo as HIG da Apple, adaptadas ao desktop:

- **Duas camadas.** O conteúdo (mensagens) é opaco; os controles flutuam sobre ele em
  pastilhas de vidro fosco. Vidro só na camada funcional, nunca no conteúdo.
- **Tipografia.** Corpo de 13 pt na interface e 14 pt nas mensagens, mínimo de 10 pt.
  A fonte e o tamanho vêm do sistema, com a Inter como reserva.
- **Cor.** Uma cor de destaque só, lida do sistema (`kdeglobals`), e verde/amarelo/vermelho
  reservados para presença.
- **Movimento.** Respeita o fator de animação do Plasma: em zero, sem transições.
- **Translucidez** pode ser desligada nos ajustes; as superfícies viram opacas.

## Estado

Funcionando: tela de entrada, conversa, menu global, bandeja, notificações, segundo plano,
vidro fosco, tema e fonte do sistema, pt-BR e inglês.

Na conversa: responder, editar no lugar, apagar, fixar e reagir (pastilha ao passar o mouse e
menu de contexto), emoji colorido com busca nos dois idiomas, figurinhas do servidor —
animadas inclusive — e menção destacada em amarelo.

Na mídia: imagem com miniatura e visualizador (zoom no ponteiro, arraste, 1:1, navegação),
vídeo tocando dentro da mensagem com linha do tempo, áudio com forma de onda que também é o
cursor, moderação sensível embaçada até o clique, download para uma pasta fixa ou perguntando
sempre, e gravação de recado de voz pelo microfone.

Nos contadores: menções somam no ícone da bandeja e na barra de tarefas, não lido acende sem
número, e tudo isso se desliga nos ajustes.

Pendente: a instância em `papo-backend.onrender.com` responde **500 em `POST /auth/register`**
(as leituras vão bem; a falha é na transação que insere em `users`/`user_settings`), então o
caminho depois do login ainda não foi exercitado contra ela. Voz e vídeo (WebRTC) não
começaram.
