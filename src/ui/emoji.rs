//! Seletor de emoji: o conjunto unicode mais usado, com busca em dois
//! idiomas, mais os emojis custom do servidor.
//!
//! A fonte de emoji embutida é monocromática — o egui desenha glifos, não
//! camadas de cor —, então o desenho sai em traço. Os emojis do servidor são
//! imagens e aparecem coloridos.

use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, UiBuilder, Vec2};

use crate::i18n::Strings;
use crate::media::MediaStore;
use crate::state::{Emoji, Store};

use super::theme::{radius, space, text, Tokens};

pub struct Group {
    pub icon: &'static str,
    pub emojis: &'static [(&'static str, &'static str)],
}

/// Cada emoji vem com as palavras que o encontram na busca (pt + en).
pub static GROUPS: &[Group] = &[
    Group {
        icon: egui_phosphor::regular::SMILEY,
        emojis: &[
            ("😀", "feliz sorriso grin happy smile"),
            ("😃", "feliz sorriso smile happy"),
            ("😄", "feliz risada smile laugh"),
            ("😁", "sorriso grin beam"),
            ("😆", "risada rindo laugh squint"),
            ("😅", "alivio suor sweat laugh"),
            ("🤣", "rolando de rir rofl rolling"),
            ("😂", "chorando de rir joy tears"),
            ("🙂", "sorriso leve slight smile"),
            ("🙃", "de cabeca para baixo upside down"),
            ("😉", "piscada wink"),
            ("😊", "envergonhado blush feliz"),
            ("😇", "anjo inocente angel halo"),
            ("🥰", "amor coracoes love hearts"),
            ("😍", "apaixonado heart eyes"),
            ("😘", "beijo kiss"),
            ("😗", "beijo kissing"),
            ("😋", "delicia yum lingua"),
            ("😜", "lingua piscando tongue wink"),
            ("🤪", "louco zany goofy"),
            ("🤨", "sobrancelha raised eyebrow duvida"),
            ("🧐", "monoculo monocle analisando"),
            ("🤓", "nerd oculos geek"),
            ("😎", "oculos de sol cool sunglasses"),
            ("🥳", "festa party comemorar"),
            ("😏", "sorriso malicioso smirk"),
            ("😒", "sem graca unamused chateado"),
            ("😞", "decepcionado disappointed"),
            ("😔", "pensativo triste pensive"),
            ("😟", "preocupado worried"),
            ("😢", "chorando cry triste"),
            ("😭", "chorando muito sob loud cry"),
            ("😤", "bufando triumph raiva"),
            ("😠", "bravo angry"),
            ("😡", "furioso rage pouting"),
            ("🤬", "xingando cursing simbolos"),
            ("🤯", "explodindo mind blown"),
            ("😳", "surpreso flushed vergonha"),
            ("🥵", "calor hot quente"),
            ("🥶", "frio cold gelado"),
            ("😱", "medo scream grito"),
            ("😨", "assustado fearful"),
            ("😰", "ansioso anxious suor"),
            ("🤗", "abraco hug"),
            ("🤔", "pensando thinking"),
            ("🤫", "silencio shush quieto"),
            ("🤭", "risadinha hand over mouth"),
            ("😬", "constrangido grimace"),
            ("🙄", "revirando os olhos eye roll"),
            ("😴", "dormindo sleeping zzz"),
            ("🥱", "bocejo yawn sono"),
            ("😷", "mascara mask doente"),
            ("🤒", "febre sick termometro"),
            ("🤮", "vomitando vomit enjoo"),
            ("🥴", "tonto woozy"),
            ("😵", "atordoado dizzy"),
            ("🤠", "caubói cowboy"),
            ("🤡", "palhaco clown"),
            ("👻", "fantasma ghost"),
            ("💀", "caveira skull morto"),
            ("👽", "alien et"),
            ("🤖", "robo robot bot"),
            ("💩", "coco poop"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::HAND,
        emojis: &[
            ("👍", "joia like polegar thumbs up"),
            ("👎", "nao curti thumbs down"),
            ("👌", "ok perfeito"),
            ("🤌", "dedos italiano pinched"),
            ("✌️", "paz victory v"),
            ("🤞", "dedos cruzados fingers crossed"),
            ("🤟", "amo love you"),
            ("🤘", "rock chifres horns"),
            ("🤙", "me liga call me"),
            ("👋", "tchau oi wave"),
            ("🙌", "maos ao alto raising hands"),
            ("👏", "palmas clap aplausos"),
            ("🙏", "obrigado por favor pray"),
            ("🤝", "aperto de maos handshake acordo"),
            ("💪", "forca muscle biceps"),
            ("🫡", "continencia salute"),
            ("🖐️", "mao aberta hand"),
            ("✊", "punho fist"),
            ("👊", "soco punch"),
            ("🫶", "coracao com as maos heart hands"),
            ("👀", "olhos eyes olhando"),
            ("🧠", "cerebro brain"),
            ("🦾", "braco mecanico"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::HEART,
        emojis: &[
            ("❤️", "coracao vermelho red heart amor"),
            ("🧡", "coracao laranja orange heart"),
            ("💛", "coracao amarelo yellow heart"),
            ("💚", "coracao verde green heart"),
            ("💙", "coracao azul blue heart"),
            ("💜", "coracao roxo purple heart"),
            ("🖤", "coracao preto black heart"),
            ("🤍", "coracao branco white heart"),
            ("💔", "coracao partido broken heart"),
            ("💖", "coracao brilhante sparkling heart"),
            ("💕", "dois coracoes two hearts"),
            ("💘", "flechado cupid"),
            ("💯", "cem 100 perfeito"),
            ("🔥", "fogo fire top"),
            ("✨", "brilho sparkles"),
            ("⭐", "estrela star"),
            ("🌟", "estrela brilhante glowing star"),
            ("💥", "explosao boom"),
            ("⚡", "raio zap energia"),
            ("🎉", "festa party popper comemoracao"),
            ("🎊", "confete confetti"),
            ("🏆", "trofeu trophy"),
            ("🥇", "primeiro lugar gold medal"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::PAW_PRINT,
        emojis: &[
            ("🐶", "cachorro dog"),
            ("🐱", "gato cat"),
            ("🐭", "rato mouse"),
            ("🐹", "hamster"),
            ("🐰", "coelho rabbit"),
            ("🦊", "raposa fox"),
            ("🐻", "urso bear"),
            ("🐼", "panda"),
            ("🐨", "coala koala"),
            ("🐯", "tigre tiger"),
            ("🦁", "leao lion"),
            ("🐮", "vaca cow"),
            ("🐷", "porco pig"),
            ("🐸", "sapo frog"),
            ("🐵", "macaco monkey"),
            ("🐔", "galinha chicken"),
            ("🐧", "pinguim penguin"),
            ("🦆", "pato duck"),
            ("🦉", "coruja owl"),
            ("🐝", "abelha bee"),
            ("🦋", "borboleta butterfly"),
            ("🐢", "tartaruga turtle"),
            ("🐍", "cobra snake"),
            ("🐙", "polvo octopus"),
            ("🐬", "golfinho dolphin"),
            ("🐳", "baleia whale"),
            ("🌵", "cacto cactus"),
            ("🌲", "arvore tree"),
            ("🌻", "girassol sunflower"),
            ("🌹", "rosa rose"),
            ("🌈", "arco iris rainbow"),
            ("☀️", "sol sun"),
            ("🌙", "lua moon"),
            ("☁️", "nuvem cloud"),
            ("🌧️", "chuva rain"),
            ("❄️", "neve snow frio"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::HAMBURGER,
        emojis: &[
            ("🍎", "maca apple"),
            ("🍌", "banana"),
            ("🍇", "uva grapes"),
            ("🍓", "morango strawberry"),
            ("🍉", "melancia watermelon"),
            ("🥑", "abacate avocado"),
            ("🍕", "pizza"),
            ("🍔", "hamburguer burger"),
            ("🌭", "cachorro quente hot dog"),
            ("🍟", "batata frita fries"),
            ("🌮", "taco"),
            ("🍣", "sushi"),
            ("🍜", "lamen ramen macarrao"),
            ("🍚", "arroz rice"),
            ("🥐", "croissant pao"),
            ("🍞", "pao bread"),
            ("🧀", "queijo cheese"),
            ("🍪", "biscoito cookie"),
            ("🍰", "bolo cake"),
            ("🍫", "chocolate"),
            ("🍿", "pipoca popcorn"),
            ("☕", "cafe coffee"),
            ("🍺", "cerveja beer"),
            ("🍷", "vinho wine"),
            ("🧉", "chimarrao mate"),
            ("🥤", "refrigerante soda"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::GAME_CONTROLLER,
        emojis: &[
            ("⚽", "futebol soccer bola"),
            ("🏀", "basquete basketball"),
            ("🏐", "volei volleyball"),
            ("🎾", "tenis tennis"),
            ("🏓", "ping pong tenis de mesa"),
            ("🎮", "videogame controle game"),
            ("🕹️", "joystick arcade"),
            ("🎲", "dado dice"),
            ("🎯", "alvo dardo target"),
            ("🎸", "guitarra guitar"),
            ("🎹", "teclado piano"),
            ("🥁", "bateria drum"),
            ("🎤", "microfone mic karaoke"),
            ("🎧", "fone headphone"),
            ("🎬", "cinema clapper filme"),
            ("🎨", "arte paint pintura"),
            ("📷", "camera foto"),
            ("🚴", "bicicleta bike"),
            ("🏃", "correndo running"),
            ("🏋️", "academia lifting"),
            ("🧘", "meditando yoga"),
            ("🏆", "trofeu troféu"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::AIRPLANE,
        emojis: &[
            ("🚗", "carro car"),
            ("🚕", "taxi"),
            ("🚌", "onibus bus"),
            ("🚑", "ambulancia"),
            ("🚓", "policia police"),
            ("🚲", "bicicleta bicycle"),
            ("🛵", "moto scooter"),
            ("✈️", "aviao plane"),
            ("🚀", "foguete rocket lancamento"),
            ("🛸", "disco voador ufo"),
            ("🚂", "trem train"),
            ("⛵", "veleiro boat"),
            ("🏠", "casa house"),
            ("🏢", "predio office"),
            ("🏥", "hospital"),
            ("🌍", "mundo terra earth"),
            ("🗺️", "mapa map"),
            ("🏖️", "praia beach"),
            ("⛰️", "montanha mountain"),
            ("🌃", "cidade noite night city"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::DESKTOP,
        emojis: &[
            ("💻", "notebook laptop computador"),
            ("🖥️", "computador desktop pc"),
            ("⌨️", "teclado keyboard"),
            ("🖱️", "mouse"),
            ("📱", "celular phone"),
            ("💾", "disquete save"),
            ("💿", "cd disco"),
            ("🔌", "tomada plug"),
            ("🔋", "bateria battery"),
            ("💡", "ideia lampada bulb"),
            ("🔍", "lupa busca search"),
            ("🔒", "cadeado lock"),
            ("🔑", "chave key"),
            ("🔨", "martelo hammer"),
            ("🛠️", "ferramentas tools"),
            ("⚙️", "engrenagem gear config"),
            ("🧪", "tubo de ensaio test"),
            ("📦", "caixa package pacote"),
            ("📄", "documento document"),
            ("📊", "grafico chart"),
            ("📌", "alfinete pin"),
            ("📎", "clipe clip anexo"),
            ("✏️", "lapis pencil editar"),
            ("📚", "livros books"),
            ("💰", "dinheiro money"),
            ("🎁", "presente gift"),
            ("🔔", "sino bell notificacao"),
            ("⏰", "despertador alarm"),
            ("⌛", "ampulheta hourglass"),
        ],
    },
    Group {
        icon: egui_phosphor::regular::CHECK_CIRCLE,
        emojis: &[
            ("✅", "certo check ok"),
            ("❌", "errado x cross"),
            ("❗", "exclamacao importante"),
            ("❓", "interrogacao duvida question"),
            ("⚠️", "aviso warning cuidado"),
            ("🚫", "proibido forbidden"),
            ("♻️", "reciclar recycle"),
            ("🔴", "vermelho red circle"),
            ("🟢", "verde green circle"),
            ("🔵", "azul blue circle"),
            ("🟡", "amarelo yellow circle"),
            ("🟣", "roxo purple"),
            ("⬛", "preto black"),
            ("⬜", "branco white"),
            ("🔺", "triangulo vermelho"),
            ("🔻", "triangulo para baixo"),
            ("➡️", "seta direita arrow right"),
            ("⬅️", "seta esquerda arrow left"),
            ("⬆️", "seta cima arrow up"),
            ("⬇️", "seta baixo arrow down"),
            ("🔄", "atualizar refresh"),
            ("♾️", "infinito infinity"),
            ("™️", "marca registrada tm"),
        ],
    },
];

/// Emojis sugeridos na barra rápida da pastilha de ações.
pub const QUICK: [&str; 6] = ["👍", "❤️", "😂", "🎉", "😮", "😢"];

/// Desenha o seletor e devolve o emoji escolhido.
pub fn picker(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    store: &Store,
    media: &mut MediaStore,
    query: &mut String,
    group: &mut usize,
    // `custom_only`: só os emojis do servidor, que fazem as vezes de figurinha.
    custom_only: bool,
) -> Option<Emoji> {
    let mut chosen = None;

    ui.set_width(300.0);
    ui.add_space(space::XS);

    // Fila rápida: o que se usa quase sempre, a um clique.
    if !custom_only {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::XXS;
        for entry in QUICK {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
            }
            draw_unicode(ui, t, media, entry, rect.shrink(5.0));
            if response.clicked() {
                chosen = Some(Emoji::Unicode(entry.to_owned()));
            }
        }
    });
    ui.add_space(space::XS);
    ui.separator();
    ui.add_space(space::XS);
    }

    // Busca
    #[cfg(target_os = "android")]
    {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::hover());
        ui.painter().rect(
            rect,
            CornerRadius::same(radius::FIELD),
            t.fill_soft,
            egui::Stroke::new(1.0, t.separator),
            egui::StrokeKind::Inside,
        );
        let field_id = ui.id().with(("emoji-search", custom_only));
        let key = format!("emoji:{field_id:?}");
        let _ = crate::platform::native_field::show(
            ui.ctx(),
            &key,
            query,
            rect.shrink2(Vec2::new(space::MD, space::SM)),
            s.emoji_search,
            crate::platform::native_field::Mode::Search,
            0,
            false,
            t.label,
            t.label_tertiary,
            text::body().size,
        );
    }

    #[cfg(not(target_os = "android"))]
    {
        let field = egui::TextEdit::singleline(query)
            .hint_text(s.emoji_search)
            .desired_width(f32::INFINITY)
            .vertical_align(Align::Center)
            .margin(egui::Margin::symmetric(space::MD as i8, space::SM as i8));
        ui.add(field);
    }
    ui.add_space(space::XS);

    let filter = query.trim().to_lowercase();
    let searching = !filter.is_empty();

    // Abas das categorias
    if !searching && !custom_only {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XXS;
            for (index, entry) in GROUPS.iter().enumerate() {
                let selected = *group == index;
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                if selected || response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(radius::CONTROL),
                        if selected { t.fill_medium } else { t.fill_soft },
                    );
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    entry.icon,
                    text::icon(15.0),
                    if selected { t.accent } else { t.label_secondary },
                );
                if response.clicked() {
                    *group = index;
                }
            }
        });
        ui.add_space(space::XS);
    }

    egui::ScrollArea::vertical()
        .max_height(260.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Emojis do servidor primeiro: são a identidade da casa.
            let custom: Vec<_> = store
                .emojis
                .iter()
                .filter(|emoji| !searching || emoji.name.to_lowercase().contains(&filter))
                .cloned()
                .collect();
            if !custom.is_empty() {
                ui.label(
                    RichText::new(s.emoji_custom.to_uppercase())
                        .font(text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(space::XXS);
                grid(ui, custom.len(), |ui, index| {
                    let emoji = &custom[index];
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(radius::CONTROL),
                            t.fill_soft,
                        );
                    }
                    if let Some(texture) = media
                        .emoji(&emoji.id, emoji.blob.as_deref())
                        .and_then(|texture| texture.frame(ui.ctx()))
                    {
                        let id = texture.id();
                        ui.painter().image(
                            id,
                            rect.shrink(4.0),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if response.on_hover_text(&emoji.name).clicked() {
                        chosen = Some(Emoji::Custom(emoji.id.clone()));
                    }
                });
                ui.add_space(space::SM);
            }

            let entries: Vec<&(&str, &str)> = if custom_only {
                Vec::new()
            } else if searching {
                GROUPS
                    .iter()
                    .flat_map(|entry| entry.emojis.iter())
                    .filter(|(_, keywords)| keywords.contains(&filter))
                    .collect()
            } else {
                GROUPS[(*group).min(GROUPS.len() - 1)].emojis.iter().collect()
            };

            if entries.is_empty() && custom.is_empty() {
                ui.add_space(space::LG);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(s.emoji_none)
                            .font(text::footnote())
                            .color(t.label_tertiary),
                    );
                });
                return;
            }

            grid(ui, entries.len(), |ui, index| {
                let (emoji, keywords) = entries[index];
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
                }
                draw_unicode(ui, t, media, emoji, rect.shrink(5.0));
                let name = keywords.split_whitespace().next().unwrap_or(emoji);
                if response.on_hover_text(name).clicked() {
                    chosen = Some(Emoji::Unicode((*emoji).to_owned()));
                }
            });
        });

    chosen
}

/// Grade de 8 colunas; o egui não tem um layout de grade fluida.
fn grid(ui: &mut egui::Ui, count: usize, mut cell: impl FnMut(&mut egui::Ui, usize)) {
    const COLUMNS: usize = 8;
    let mut index = 0;
    while index < count {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XXS;
            for _ in 0..COLUMNS {
                if index >= count {
                    break;
                }
                cell(ui, index);
                index += 1;
            }
        });
    }
}

/// Uma reação desenhada: emoji unicode ou a imagem do emoji custom.
pub fn draw_reaction(
    ui: &mut egui::Ui,
    t: &Tokens,
    media: &mut MediaStore,
    store: &Store,
    emoji: &Emoji,
    rect: egui::Rect,
) {
    match emoji {
        Emoji::Unicode(text) => draw_unicode(ui, t, media, text, rect),
        Emoji::Custom(id) => {
            let blob = store
                .emojis
                .iter()
                .find(|emoji| &emoji.id == id)
                .and_then(|emoji| emoji.blob.clone());
            if let Some(texture) = media
                .emoji(id, blob.as_deref())
                .and_then(|texture| texture.frame(ui.ctx()))
            {
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(radius::CONTROL),
                    Stroke::new(1.0, t.separator),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

/// Um emoji unicode: imagem colorida quando o sistema tem a fonte, glifo em
/// traço quando não tem.
pub fn draw_unicode(
    ui: &mut egui::Ui,
    t: &Tokens,
    media: &mut MediaStore,
    emoji: &str,
    rect: egui::Rect,
) {
    if let Some(texture) = media.unicode_emoji(ui.ctx(), emoji) {
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        return;
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        emoji,
        egui::FontId::proportional(rect.height() * 0.85),
        t.label,
    );
}

/// Largura que a reação ocupa; o emoji custom é quadrado.
pub fn reaction_width(emoji: &Emoji) -> f32 {
    match emoji {
        Emoji::Unicode(_) => 16.0,
        Emoji::Custom(_) => 16.0,
    }
}

/// Espaço reservado a um popup, alinhado à âncora sem sair da janela.
pub fn popup_area(ui: &egui::Ui, anchor: egui::Rect, size: Vec2) -> egui::Rect {
    let screen = ui.ctx().viewport_rect();
    let mut min = egui::pos2(anchor.min.x, anchor.max.y + space::XS);
    if min.y + size.y > screen.max.y - space::MD {
        min.y = (anchor.min.y - size.y - space::XS).max(screen.min.y + space::MD);
    }
    if min.x + size.x > screen.max.x - space::MD {
        min.x = (screen.max.x - size.x - space::MD).max(screen.min.x + space::MD);
    }
    egui::Rect::from_min_size(min, size)
}

/// Moldura de um popup flutuante (seletor de emoji, menu de contexto).
pub fn popup_frame(ui: &mut egui::Ui, t: &Tokens, rect: egui::Rect, build: impl FnOnce(&mut egui::Ui)) {
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET),
        t.elevated_bg,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink(space::SM))
            .layout(Layout::top_down(Align::Min)),
        build,
    );
}

// ---------------------------------------------------------------------------
// Texto com emoji
// ---------------------------------------------------------------------------

/// Um pedaço de mensagem: texto corrido, emoji unicode ou emoji do servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text(String),
    Unicode(String),
    Custom(String),
}

/// Quebra o texto em pedaços, reconhecendo emoji unicode e `:apelido:` dos
/// emojis do servidor.
pub fn tokenize(text: &str, custom: &[crate::state::CustomEmoji]) -> Vec<Token> {
    use super::emoji_raster::is_emoji;

    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;

    while index < chars.len() {
        let c = chars[index];

        // :apelido: de emoji do servidor.
        if c == ':'
            && let Some(end) = chars[index + 1..]
                .iter()
                .position(|c| *c == ':')
                .map(|offset| index + 1 + offset)
        {
                let name: String = chars[index + 1..end].iter().collect();
                let known = custom.iter().find(|emoji| emoji.name == name);
                if let Some(emoji) = known {
                    if !buffer.is_empty() {
                        tokens.push(Token::Text(std::mem::take(&mut buffer)));
                    }
                    tokens.push(Token::Custom(emoji.id.clone()));
                    index = end + 1;
                    continue;
                }
        }

        if is_emoji(c) {
            if !buffer.is_empty() {
                tokens.push(Token::Text(std::mem::take(&mut buffer)));
            }
            // A sequência inteira é um emoji só: junta modificadores, ZWJ e
            // seletores de variação.
            let mut cluster = String::new();
            while index < chars.len() {
                let c = chars[index];
                let code = c as u32;
                let part = is_emoji(c)
                    || code == 0x200D
                    || (0x1F3FB..=0x1F3FF).contains(&code)
                    || code == 0x20E3;
                if !part {
                    break;
                }
                cluster.push(c);
                index += 1;
                // Depois de um ZWJ vem sempre mais um pedaço.
                if chars.get(index).map(|c| *c as u32) == Some(0x200D) {
                    continue;
                }
            }
            tokens.push(Token::Unicode(cluster));
            continue;
        }

        buffer.push(c);
        index += 1;
    }

    if !buffer.is_empty() {
        tokens.push(Token::Text(buffer));
    }
    tokens
}

/// Mensagem só de emoji, no máximo três: merece aparecer grande.
pub fn jumbo(tokens: &[Token]) -> bool {
    let emojis = tokens
        .iter()
        .filter(|token| matches!(token, Token::Unicode(_) | Token::Custom(_)))
        .count();
    let only_emoji = tokens.iter().all(|token| match token {
        Token::Text(text) => text.trim().is_empty(),
        _ => true,
    });
    only_emoji && (1..=3).contains(&emojis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separa_texto_de_emoji() {
        let tokens = tokenize("oi 👋 tudo bem?", &[]);
        assert_eq!(
            tokens,
            vec![
                Token::Text("oi ".into()),
                Token::Unicode("👋".into()),
                Token::Text(" tudo bem?".into()),
            ]
        );
    }

    #[test]
    fn junta_sequencia_com_zwj() {
        let tokens = tokenize("👩‍💻", &[]);
        assert_eq!(tokens, vec![Token::Unicode("👩\u{200d}💻".into())]);
    }

    #[test]
    fn reconhece_emoji_do_servidor() {
        let custom = vec![crate::state::CustomEmoji {
            id: "e-1".into(),
            name: "papo".into(),
            blob: None,
        }];
        let tokens = tokenize("manda :papo: aí", &custom);
        assert_eq!(
            tokens,
            vec![
                Token::Text("manda ".into()),
                Token::Custom("e-1".into()),
                Token::Text(" aí".into()),
            ]
        );
    }

    #[test]
    fn dois_pontos_solto_continua_texto() {
        let tokens = tokenize("horário: 10:30", &[]);
        assert_eq!(tokens, vec![Token::Text("horário: 10:30".into())]);
    }
}
