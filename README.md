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
cargo run -- selftest <usuário> <senha>   # exercita REST: login, whoami, canais, pessoas
cargo run -- notify-test                  # dispara uma notificação de exemplo
PAPO_SERVER=http://localhost:8080 cargo run -- selftest
```

O endereço do servidor também é editável na tela de entrada e fica guardado nos ajustes.

## Como está organizado

```
src/
  api/        cliente REST, WebSocket e a ponte com a interface
    client.rs   reqwest + cookie de sessão (Auth), persistido em disco
    ws.rs       socket com heartbeat e reconexão progressiva
    net.rs      thread com runtime tokio; a janela troca comandos e updates por canais
  state/      estado da aplicação, alimentado pelas respostas e pelos eventos
  ui/         telas e sistema de design
    theme.rs    tokens (cor, tipografia, espaço) e o estilo do egui
    glass.rs    desfoque de fundo em OpenGL — o vidro fosco
    shell.rs    janela principal: canais · conversa · membros
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

## O que aprendemos sobre esta pilha

Anotado porque custou tempo e não está documentado em lugar nenhum:

- **O egui não tem menu nativo.** O menu global é implementado à mão: um serviço
  `com.canonical.dbusmenu` e o protocolo `org_kde_kwin_appmenu` apontando para ele. No X11 o
  caminho é outro (propriedades `_KDE_NET_WM_APPMENU_*` e o registrador do KDE).
- **Desfoque de verdade precisa ser nosso.** O compositor só desfoca o que está *atrás da
  janela*. Para uma barra embaçar as mensagens que passam por baixo dela, o desfoque tem que
  acontecer no nosso renderizador: `ui/glass.rs` copia o framebuffer, borra em resolução
  reduzida e devolve com cantos arredondados. Duas armadilhas: o egui deixa o *scissor*
  recortado no retângulo da callback (zera as passagens fora da tela) e a cópia lê do
  `READ_FRAMEBUFFER`, que não é o que o egui tem ligado para desenhar.
- **O KWin 6.7 não anuncia mais `org_kde_kwin_blur_manager`** para clientes Wayland, mesmo
  com o efeito ligado. `platform/blur.rs` continua no lugar e se desliga sozinho.
- **No Wayland a janela não se esconde nem se levanta.** No winit 0.30, `set_visible` não faz
  nada, `set_minimized(false)` é ignorado e `focus_window` é vazio. Fechar minimiza, e quem
  traz a janela de volta é um script carregado no KWin (`platform/kwin.rs`), a mesma técnica
  do kdotool. O xdg-activation sozinho só faz a barra de tarefas piscar.
- **Emoji colorido não renderiza** no egui (fontes CBDT/COLR); usamos a Noto Emoji
  monocromática. Os ícones do Phosphor precisam de uma família só deles: a fonte de ícones que
  o egui já traz ocupa a mesma faixa de uso privado e vence a disputa pelos glifos.

## Estado

Funcionando: tela de entrada, casca da conversa, menu global, bandeja, notificações, segundo
plano, vidro fosco, tema e fonte do sistema, pt-BR e inglês.

Pendente: a instância em `papo-backend.onrender.com` responde **500 em `POST /auth/register`**
(as leituras vão bem; a falha é na transação que insere em `users`/`user_settings`), então o
caminho depois do login ainda não foi exercitado contra ela. Voz e vídeo (WebRTC) não
começaram.
