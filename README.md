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
cargo run -- voice-sdp                    # imprime a oferta SDP da call, sem rede
cargo run -- voice-test <usuário> <senha> # entra numa call de verdade, sem janela
PAPO_SERVER=http://localhost:8080 cargo run -- selftest
```

O `demo` é o modo de trabalho para a interface: povoa canais, pessoas e mensagens sem
rede e **gera a mídia de verdade** (imagem, vídeo Theora e áudio Ogg) direto no cache de
anexos, então imagem, vídeo, áudio, reações e figurinhas passam pelo mesmo caminho que
passariam vindos do servidor.

O endereço do servidor é editável na tela de entrada e fica guardado nos ajustes.

## Instalando

```bash
./scripts/deps.sh --install     # pacotes de sistema da sua distribuição
cargo install --locked --path . # o binário em ~/.cargo/bin
./scripts/install-desktop.sh    # .desktop, ícones e metadados no ~/.local/share
```

Para reproduzir o CI antes de empurrar (o clippy do portão tem versão fixa,
em `.github/workflows/ci.yml`):

```bash
cargo +1.98.0 clippy --locked --all-targets -- -D warnings
cargo +1.95.0 build --locked   # o mínimo declarado no Cargo.toml
```

`scripts/deps.sh` conhece apt, dnf e pacman; sem `--install` ele só mostra o
comando. Os nomes do apt são conferidos no CI a cada push, então não envelhecem
calados. O binário liga o GStreamer dinamicamente: mesmo quem baixa o tarball
precisa dos pacotes de execução (`./deps.sh --runtime`).

O `--locked` não é opcional: sem ele o cargo reresolve as dependências para a
última versão compatível, e alguma delas vai exigir um rustc mais novo que o
seu. O mínimo é **rustc 1.95** (o do egui), e o CI compila com essa versão
exata para que ele não suba sem querer.

Empacotado, por ordem de preferência:

| Formato | Arquivo | Para quem |
| --- | --- | --- |
| Flatpak | `packaging/io.github.chriexpe.Papo.yml` | qualquer distribuição; o runtime já traz GStreamer e codecs |
| Bundle Flatpak | `papo-<versão>-<arco>.flatpak` no release | instalar sem Flathub, com `flatpak install --user ./papo-*.flatpak` |
| `.deb` | `packaging/deb/build.sh` | Debian, Ubuntu, Zorin e derivados; as dependências vêm no pacote |
| Arch/AUR | `packaging/PKGBUILD` | Arch e derivados |
| Tarball | gerado pelo workflow `release` | quem só quer descompactar e rodar |

Todos saem do mesmo workflow `release` a cada etiqueta `v*`, em x86_64 e
aarch64. O bundle não é assinado (um `.flatpak` avulso não exige assinatura) e
não se atualiza sozinho: quem quiser atualizações automáticas usa o Flathub,
que assina o repositório com a chave dele.

O Flatpak precisa das dependências do cargo em disco antes de compilar (a
compilação roda sem rede): rode `./scripts/flatpak-sources.sh` sempre que o
`Cargo.lock` mudar, depois `flatpak-builder build packaging/io.github.chriexpe.Papo.yml`.

O binário liga o GStreamer dinamicamente. Instalado de tarball ou pelo cargo, ele
depende do GStreamer do sistema (`gst-plugins-base`, `-good`, `-bad` e `gst-libav`);
no Flatpak isso vem do runtime.

Só Linux por enquanto: bandeja, menu global, notificações, desfoque e restauração
de janela são todos D-Bus e Wayland/X11. As arquiteturas publicadas são x86_64 e
aarch64, compiladas cada uma na sua máquina no `release.yml`.

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
  voice/      a call: WebRTC pelo webrtcbin, sinalizada pelo mesmo socket
    engine.rs   o pipeline e a máquina de sinalização, numa thread própria
    slots.rs    os lugares de vídeo do SFU, espelhados para dar nome ao vídeo
    ice.rs      STUN e TURN do backend no formato que o webrtcbin aceita
  ui/         telas e sistema de design
    rail.rs     trilho de servidores, a coluna de ícones à esquerda de tudo
    headerbar.rs  barra de título própria, onde não há menu global
    theme.rs    tokens (cor, tipografia, espaço) e o estilo do egui
    glass.rs    desfoque de fundo em OpenGL — o vidro fosco
    shell.rs    janela principal: canais · conversa · membros
    attachments.rs  imagem, vídeo, áudio e arquivo dentro da mensagem
    viewer.rs   tela cheia com zoom, arraste e navegação entre anexos
    emoji.rs    seletor, reações e o texto que mistura emoji com palavras
    emoji_raster.rs  emoji colorido tirado da fonte do sistema, via swash
    auth.rs     entrada e primeiro uso da instância
    call.rs     a call em três formas: no canal, em folha e em janela
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
- **A moldura segue a área de trabalho.** No Plasma o menu vai para o painel e a
  barra de título é do compositor. Em todo o resto — GNOME à frente, que tirou o
  menu global na 3.32 — o Papo desenha a própria barra, com o título no meio e o
  menu num hambúrguer. Sem isso não haveria caminho nenhum para ajustes, idioma
  ou sair. `PAPO_CHROME=own` e `PAPO_CHROME=system` forçam um dos dois.
- **Ajustes numa folha só.** Duas pastilhas gêmeas na coluna da esquerda — o
  servidor em cima, você embaixo — com a mesma anatomia e a engrenagem na ponta
  oposta. As duas abrem a mesma folha, que é sólida: o guia do material diz que
  superfície grande fica mais opaca, e que vidro sobre vidro desmancha a
  hierarquia. `PAPO_SHEET=app` ou `PAPO_SHEET=servidor` abre a folha já na
  partida, para trabalhar no desenho dela sem clicar até lá
  (`PAPO_SHEET=servidor:figurinhas` abre direto num painel).
- **Figurinha entra pelo arquivo, não pelo nome.** Escolhe-se a imagem, vê-se
  como vai ficar, e só então se dá o nome. O que passa de 512 px ou 256 KB é
  reduzido pelo cliente: imagem parada vira WebP sem perdas, GIF animado
  continua GIF animado, quadro a quadro. O servidor não tem como renomear
  (`/emojis` só cria e apaga), e a tela diz isso em vez de oferecer um campo
  que não salvaria.
- **`:apelido` sugere enquanto se digita.** A lista sobe da caixa de mensagem,
  seta para cima e para baixo escolhe, Enter ou Tab preenche, Esc dispensa.

## Estado

Funcionando: tela de entrada, conversa, menu global, bandeja, notificações, segundo plano,
vidro fosco, tema e fonte do sistema, pt-BR e inglês.

Vários servidores ao mesmo tempo, no trilho à esquerda: cada um com conta, sessão, canais
e mídia próprios, todos conectados de uma vez — a menção de um servidor que não está na
tela ainda acende o contador. A sessão se renova sozinha antes de vencer, e um servidor
fechado pede a senha do servidor na própria tela de entrada.

Na conversa: responder, editar no lugar, apagar, fixar e reagir (pastilha ao passar o mouse e
menu de contexto), emoji colorido com busca nos dois idiomas, figurinhas do servidor —
animadas inclusive — e menção destacada em amarelo.

Na mídia: imagem com miniatura e visualizador (zoom no ponteiro, arraste, 1:1, navegação),
vídeo tocando dentro da mensagem com linha do tempo, áudio com forma de onda que também é o
cursor, moderação sensível embaçada até o clique, download para uma pasta fixa ou perguntando
sempre, e gravação de recado de voz pelo microfone.

Nos contadores: menções somam no ícone da bandeja e na barra de tarefas, não lido acende sem
número, e tudo isso se desliga nos ajustes. O Papo também pode ser marcado para iniciar com a
sessão, por um `.desktop` em `~/.config/autostart` — dentro do Flatpak, pelo mesmo caminho,
já que o `Exec` de lá fora chama o `flatpak run`.

Na administração: canais (criar, renomear, excluir, reordenar e a notificação de cada um),
cargos com as sete permissões e quem tem cada uma, perfil (apelido, recado, presença, foto,
senha), servidor (nome, público ou fechado, figurinhas) e o registro de auditoria. As sessões
abertas da conta aparecem no perfil e dá para encerrar uma ou todas.

Na call: entrar e sair de um canal de voz, microfone com mudo, quem está falando,
câmera ligando e desligando, e a câmera dos outros aparecendo sozinha. O transporte é
WebRTC pelo `webrtcbin`; a sinalização vai pelo mesmo WebSocket do resto. O SFU do
backend é quem mistura e reenvia — ninguém fala direto com ninguém.

A call aparece de três jeitos, e passa de um para o outro sem cortar o áudio: **no
canal** quando é só voz, **numa folha de vidro** por cima da conversa assim que aparece
vídeo (encolhível numa pastilha, para continuar escrevendo), e **em janela própria**
quando se pede — para jogá-la noutro monitor.

Duas coisas dependem de mudança no backend, e por ora o cliente contorna:

- **Os lugares de vídeo são seis** (`VOICE_VIDEO_SLOTS`), e o servidor não conta ao
  cliente quantos são. A oferta precisa nascer com um número exato de linhas de
  recepção, então o cliente assume o padrão do backend (6 de vídeo, 8 de áudio). Se a
  instância mudar esses números, a conta de quem ocupa qual lugar desanda. Expor os dois
  valores no `GET /voice/ice-servers` resolveria.
- **Quem está numa sala de voz só se sabe entrando.** O `voice_joined` é unicast para
  quem entra; de fora, só chegam as mudanças (`voice_state_update`, `voice_leave`). Uma
  janela aberta depois vê a sala vazia até alguém se mexer. Faltaria a lista de estados
  de voz numa rota REST — no `GET /channels`, por exemplo.

Compartilhar tela não entrou nesta leva: o caminho de vídeo é o mesmo da câmera e o
servidor já trata os dois lados, mas falta a captura pelo portal e o lugar dela na grade.

Sem tela ainda, mas implementado contra o contrato: banner do perfil, ficha de uma pessoa só,
ajustes guardados no servidor e prévia de link.

Uma divergência entre o `openapi.yml` do backend e o que o servidor faz, descoberta
testando contra uma instância de verdade:

- O mime dos anexos é deduzido pelo servidor e erra: `.mp4` volta como
  `application/octet-stream`. O cliente cai na extensão quando o mime não diz nada, senão
  vídeo e áudio apareceriam como um arquivo qualquer.
