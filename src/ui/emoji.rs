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

#[derive(Clone, Default)]
struct PickerNavigation {
    frame: u64,
    query: String,
    group: usize,
    index: usize,
}

fn navigate(index: usize, key: egui::Key, columns: usize, custom: usize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    let index = index.min(total - 1);
    let (base, count) = if index < custom {
        (0, custom)
    } else {
        (custom, total - custom)
    };
    let local = index - base;
    let column = local % columns;
    match key {
        egui::Key::ArrowLeft => index.saturating_sub(1),
        egui::Key::ArrowRight => (index + 1).min(total - 1),
        egui::Key::ArrowUp if local >= columns => index - columns,
        egui::Key::ArrowUp if base > 0 => {
            (((custom - 1) / columns) * columns + column).min(custom - 1)
        }
        egui::Key::ArrowDown if (local / columns + 1) * columns < count => {
            base + (local + columns).min(count - 1)
        }
        egui::Key::ArrowDown if custom > 0 && base == 0 && custom < total => {
            custom + column.min(total - custom - 1)
        }
        _ => index,
    }
}

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
    let navigation_id = ui.id().with(("emoji-navigation", custom_only));
    let frame = ui.ctx().cumulative_frame_nr();
    let previous = ui.data(|data| data.get_temp::<PickerNavigation>(navigation_id));
    let opening = previous.as_ref().is_none_or(|nav| nav.frame + 1 < frame);
    let mut navigation = if opening {
        PickerNavigation::default()
    } else {
        previous.unwrap()
    };
    navigation.frame = frame;
    // Take navigation before TextEdit can move its cursor or surrender focus.
    #[allow(unused_mut)]
    let mut keys = ui.input_mut(|input| {
        let mut keys = Vec::new();
        input.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
                && modifiers.is_none()
                && matches!(
                    key,
                    egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                        | egui::Key::Enter
                )
            {
                keys.push(*key);
                return false;
            }
            true
        });
        keys
    });
    #[allow(unused_mut)]
    let mut accept = keys.contains(&egui::Key::Enter);

    ui.set_width(ui.available_width().min(300.0));
    ui.add_space(space::XS);

    // Fila rápida: o que se usa quase sempre, a um clique.
    if !custom_only {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XXS;
            for entry in QUICK {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(radius::CONTROL),
                        t.fill_soft,
                    );
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
        let events = crate::platform::native_field::show(
            ui.ctx(),
            &key,
            query,
            rect.shrink2(Vec2::new(space::MD, space::SM)),
            s.emoji_search,
            crate::platform::native_field::Mode::Search,
            0,
            opening,
            t.label,
            t.label_tertiary,
            text::body().size,
        );
        accept |= events.submit;
        keys.extend(crate::platform::native_field::take_navigation(&key));
    }

    #[cfg(not(target_os = "android"))]
    {
        let search_id = ui.id().with(("emoji-search", custom_only));
        let typing = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Ime(_)
                )
            })
        });
        if opening || typing {
            ui.memory_mut(|memory| memory.request_focus(search_id));
        }
        let field = egui::TextEdit::singleline(query)
            .id(search_id)
            .hint_text(RichText::new(s.emoji_search).font(text::body()))
            .font(text::body())
            .desired_width(f32::INFINITY)
            .vertical_align(Align::Center)
            .margin(egui::Margin::symmetric(space::MD as i8, space::SM as i8));
        ui.add(field);
        ui.memory_mut(|memory| {
            if memory.has_focus(search_id) {
                memory.set_focus_lock_filter(
                    search_id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            }
        });
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
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
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
                    if selected {
                        t.accent
                    } else {
                        t.label_secondary
                    },
                );
                if response.clicked() {
                    *group = index;
                }
            }
        });
        ui.add_space(space::XS);
    }

    let custom: Vec<_> = store
        .emojis
        .iter()
        .filter(|emoji| !searching || emoji.name.to_lowercase().contains(&filter))
        .collect();
    let entries: Vec<&(&str, &str)> = if custom_only {
        Vec::new()
    } else if searching {
        GROUPS
            .iter()
            .flat_map(|entry| entry.emojis.iter())
            .filter(|(_, keywords)| keywords.contains(&filter))
            .collect()
    } else {
        GROUPS[(*group).min(GROUPS.len() - 1)]
            .emojis
            .iter()
            .collect()
    };
    let changed = navigation.query != *query || navigation.group != *group;
    if changed {
        navigation.index = 0;
    }
    navigation.query.clone_from(query);
    navigation.group = *group;
    let total = custom.len() + entries.len();
    navigation.index = navigation.index.min(total.saturating_sub(1));
    let scroll_selected = changed || keys.iter().any(|key| *key != egui::Key::Enter);

    // The scroll viewport owns all remaining panel height. On shorter
    // windows its items must scroll, not paint beyond the popup or into chat.
    let scroll_height = ui.available_height().clamp(1.0, 260.0);
    egui::ScrollArea::vertical()
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (columns, _) = grid_geometry(ui.available_width());
            for key in &keys {
                navigation.index = navigate(navigation.index, *key, columns, custom.len(), total);
            }
            // Emojis do servidor primeiro: são a identidade da casa.
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
                    let selected = navigation.index == index;
                    if selected && scroll_selected {
                        response.scroll_to_me(Some(Align::Center));
                    }
                    if selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(radius::CONTROL),
                            if selected { t.fill_medium } else { t.fill_soft },
                        );
                    }
                    if let Some(texture) = media
                        .emoji(&emoji.id, emoji.blob.as_deref())
                        .and_then(|texture| texture.frame(ui.ctx()))
                    {
                        paint_custom_image(ui.painter(), texture, rect.shrink(4.0), Color32::WHITE);
                    }
                    if response.on_hover_text(&emoji.name).clicked() {
                        chosen = Some(Emoji::Custom(emoji.id.clone()));
                    }
                });
                ui.add_space(space::SM);
            }

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
                let selected = navigation.index == custom.len() + index;
                if selected && scroll_selected {
                    response.scroll_to_me(Some(Align::Center));
                }
                if selected || response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(radius::CONTROL),
                        if selected { t.fill_medium } else { t.fill_soft },
                    );
                }
                draw_unicode(ui, t, media, emoji, rect.shrink(5.0));
                let name = keywords.split_whitespace().next().unwrap_or(emoji);
                if response.on_hover_text(name).clicked() {
                    chosen = Some(Emoji::Unicode((*emoji).to_owned()));
                }
            });
        });

    if accept {
        chosen = if let Some(emoji) = custom.get(navigation.index) {
            Some(Emoji::Custom(emoji.id.clone()))
        } else {
            entries
                .get(navigation.index.saturating_sub(custom.len()))
                .map(|(emoji, _)| Emoji::Unicode((*emoji).to_owned()))
        };
    }
    ui.data_mut(|data| {
        if chosen.is_some() {
            data.remove::<PickerNavigation>(navigation_id);
        } else {
            data.insert_temp(navigation_id, navigation);
        }
    });
    chosen
}

/// Layout de células de 30 pt que ocupa toda a largura útil do popup.
/// O número de colunas continua responsivo; a folga vai *entre* as células,
/// não sobra como uma faixa vazia à direita das figurinhas/emojis.
fn grid_geometry(width: f32) -> (usize, f32) {
    const CELL: f32 = 30.0;
    const MAX_COLUMNS: usize = 8;
    let width = width.max(CELL);
    let columns = ((width + space::XXS) / (CELL + space::XXS))
        .floor()
        .max(1.0)
        .min(MAX_COLUMNS as f32) as usize;
    let gap = if columns > 1 {
        ((width - CELL * columns as f32) / (columns - 1) as f32).max(space::XXS)
    } else {
        0.0
    };
    (columns, gap)
}

/// Grade compartilhada pelos emojis Unicode e stickers do servidor.
fn grid(ui: &mut egui::Ui, count: usize, mut cell: impl FnMut(&mut egui::Ui, usize)) {
    let (columns, gap) = grid_geometry(ui.available_width());
    let mut index = 0;
    while index < count {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for _ in 0..columns {
                if index >= count {
                    break;
                }
                cell(ui, index);
                index += 1;
            }
        });
    }
}

#[cfg(test)]
mod picker_grid_tests {
    use super::*;

    #[test]
    fn keyboard_navigation_crosses_partial_grid_sections() {
        use egui::Key::*;
        assert_eq!(navigate(0, ArrowDown, 8, 3, 25), 3);
        assert_eq!(navigate(5, ArrowUp, 8, 3, 25), 2);
        assert_eq!(navigate(5, ArrowDown, 8, 3, 25), 13);
        assert_eq!(navigate(0, ArrowLeft, 8, 0, 25), 0);
        assert_eq!(navigate(24, ArrowRight, 8, 0, 25), 24);
        assert_eq!(navigate(24, ArrowDown, 8, 0, 25), 24);
        assert_eq!(navigate(0, ArrowDown, 8, 0, 0), 0);
    }

    #[test]
    fn picker_focuses_search_then_arrows_and_enter_select_without_sending() {
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx, None);
        let tokens = Tokens::new(super::super::theme::Appearance::Dark, None);
        let mut store = Store::default();
        store.emojis = (0..20)
            .map(|index| crate::state::CustomEmoji {
                id: index.to_string(),
                name: format!("sticker{index}"),
                blob: None,
            })
            .collect();
        let mut media = MediaStore::new(None);
        let mut query = String::new();
        let mut group = 0;
        let mut run = |events| {
            let mut chosen = None;
            let mut enter_left = false;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(300.0, 380.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    chosen = picker(
                        ui,
                        &tokens,
                        crate::i18n::Lang::En.strings(),
                        &store,
                        &mut media,
                        &mut query,
                        &mut group,
                        true,
                    );
                    enter_left = ui.input(|input| input.key_pressed(egui::Key::Enter));
                },
            );
            output.textures_delta.clear();
            assert!(!enter_left, "Enter must belong to the picker");
            (chosen, query.clone())
        };
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        run(vec![]);
        assert_eq!(run(vec![egui::Event::Text("sticker".into())]).1, "sticker");
        run(vec![key(egui::Key::ArrowDown), key(egui::Key::ArrowRight)]);
        assert_eq!(
            run(vec![key(egui::Key::Enter)]).0,
            Some(Emoji::Custom("9".into()))
        );
        // Reopening resets focus/selection; empty results cannot select a stale item.
        assert_eq!(
            run(vec![
                egui::Event::Text("missing".into()),
                key(egui::Key::Enter)
            ])
            .0,
            None
        );
    }

    #[test]
    fn grid_spreads_eight_icons_across_the_available_width() {
        let (columns, gap) = grid_geometry(300.0);
        assert_eq!(columns, 8);
        assert!(gap > space::XXS);
        let occupied = columns as f32 * 30.0 + (columns - 1) as f32 * gap;
        assert!((occupied - 300.0).abs() < 0.01);
    }

    #[test]
    fn grid_adapts_to_narrower_scroll_viewport() {
        for width in [120.0, 180.0, 230.0, 280.0, 300.0] {
            let (columns, gap) = grid_geometry(width);
            assert!((1..=8).contains(&columns));
            let occupied = columns as f32 * 30.0 + (columns - 1) as f32 * gap;
            assert!((occupied - width).abs() < 0.01, "width {width}");
        }
    }
}

/// Encaixa a imagem inteira no espaço disponível sem alterar sua proporção.
pub(super) fn paint_custom_image(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    bounds: egui::Rect,
    tint: Color32,
) {
    let natural = texture.size_vec2();
    let scale = (bounds.width() / natural.x).min(bounds.height() / natural.y);
    let rect = egui::Rect::from_center_size(bounds.center(), natural * scale);
    painter.image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        tint,
    );
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
                paint_custom_image(ui.painter(), texture, rect, Color32::WHITE);
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

/// Desenha um emoji usado dentro de nome de canal.
///
/// Em modo monocromático, Unicode volta ao glifo vetorial do egui e emoji
/// custom ganha uma textura em tons de cinza, tingida exatamente com a cor
/// do rótulo. Fora dele, usa o mesmo caminho colorido das mensagens.
pub fn draw_channel_name_emoji(
    ui: &mut egui::Ui,
    t: &Tokens,
    media: &mut MediaStore,
    store: &Store,
    emoji: &Emoji,
    rect: egui::Rect,
    monochrome: bool,
    color: Color32,
) {
    if !monochrome {
        draw_reaction(ui, t, media, store, emoji, rect);
        return;
    }

    match emoji {
        Emoji::Unicode(text) => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(rect.height() * 0.85),
                color,
            );
        }
        Emoji::Custom(id) => {
            let Some(custom) = store.emojis.iter().find(|custom| &custom.id == id) else {
                return;
            };
            let Some(blob) = custom.blob.as_deref() else {
                return;
            };
            if let Some(texture) = channel_custom_mono_texture(ui.ctx(), id, blob) {
                paint_custom_image(ui.painter(), &texture, rect, color);
            }
        }
    }
}

fn channel_custom_mono_texture(
    ctx: &egui::Context,
    id: &str,
    blob: &str,
) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::new(("channel-name-mono-emoji", id));
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(cache_id)) {
        return Some(texture);
    }

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob.as_bytes())
        .ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    let decoded = if decoded.width().max(decoded.height()) > 96 {
        decoded.resize(96, 96, image::imageops::FilterType::CatmullRom)
    } else {
        decoded
    };
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = rgba
        .pixels()
        .map(|pixel| {
            let [r, g, b, a] = pixel.0;
            // Luminância preserva os detalhes internos; a textura cinza é
            // tingida no draw com a mesma cor do nome do canal.
            let luma = ((r as u16 * 54 + g as u16 * 183 + b as u16 * 19) / 256) as u8;
            Color32::from_rgba_unmultiplied(luma, luma, luma, a)
        })
        .collect();
    let image = egui::ColorImage {
        size,
        source_size: egui::Vec2::new(size[0] as f32, size[1] as f32),
        pixels,
    };
    let texture = ctx.load_texture(
        format!("channel-name-mono:{id}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(cache_id, texture.clone()));
    Some(texture)
}

/// Divide só os emojis no começo do nome. Eles continuam sendo parte do
/// próprio `name` no backend; isto é apenas apresentação.
pub fn channel_name_leading(
    name: &str,
    custom: &[crate::state::CustomEmoji],
) -> (Vec<Emoji>, String) {
    let mut leading = Vec::new();
    let mut text = String::new();
    let mut prefix = true;

    for token in tokenize(name, custom) {
        if prefix {
            match token {
                Token::Unicode(value) => leading.push(Emoji::Unicode(value)),
                Token::Custom(id) => leading.push(Emoji::Custom(id)),
                Token::Text(value) if value.trim().is_empty() => {}
                Token::Text(value) => {
                    prefix = false;
                    text.push_str(value.trim_start());
                }
            }
        } else {
            match token {
                Token::Text(value) | Token::Unicode(value) => text.push_str(&value),
                Token::Custom(id) => {
                    if let Some(custom) = custom.iter().find(|custom| custom.id == id) {
                        text.push(':');
                        text.push_str(&custom.name);
                        text.push(':');
                    }
                }
            }
        }
    }

    (leading, text)
}

/// Desenha o nome de canal depois do #/alto-falante, mantendo o emoji dentro
/// do nome e apenas escolhendo se ele aparece monocromático ou original.
#[allow(clippy::too_many_arguments)]
pub fn draw_channel_name(
    ui: &mut egui::Ui,
    t: &Tokens,
    media: &mut MediaStore,
    store: &Store,
    name: &str,
    pos: egui::Pos2,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
    monochrome: bool,
    uppercase: bool,
) -> egui::Rect {
    let (leading, mut label) = channel_name_leading(name, &store.emojis);
    if uppercase {
        label = label.to_uppercase();
    }

    let icon_side = font.size.max(13.0);
    let icon_gap = 3.0;
    let icons_width = if leading.is_empty() {
        0.0
    } else {
        leading.len() as f32 * (icon_side + icon_gap)
    };
    let text_width = (max_width - icons_width).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(label, font, color);
    job.wrap.max_width = text_width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);

    let total_width = (icons_width + galley.size().x).min(max_width.max(0.0));
    let height = galley.size().y.max(icon_side);
    let rect = egui::Rect::from_min_size(
        egui::pos2(pos.x, pos.y - height / 2.0),
        egui::vec2(total_width, height),
    );
    let painter = ui.painter().with_clip_rect(egui::Rect::from_min_size(
        egui::pos2(pos.x, rect.min.y),
        egui::vec2(max_width.max(0.0), height),
    ));

    let mut x = pos.x;
    for emoji in &leading {
        let art = egui::Rect::from_center_size(
            egui::pos2(x + icon_side / 2.0, pos.y),
            egui::Vec2::splat(icon_side),
        );
        // draw_channel_name_emoji uses ui's painter; apply the same clip by
        // temporarily narrowing the UI clip.
        let old_clip = ui.clip_rect();
        ui.set_clip_rect(painter.clip_rect());
        draw_channel_name_emoji(ui, t, media, store, emoji, art, monochrome, color);
        ui.set_clip_rect(old_clip);
        x += icon_side + icon_gap;
    }
    painter.galley(
        egui::pos2(x, pos.y - galley.size().y / 2.0),
        galley,
        color,
    );
    rect
}

/// Largura reservada à reação; a imagem cabe nela mantendo sua proporção.
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
    let inner = rect.shrink(space::SM);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(inner));
            build(ui);
        },
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
    use unicode_segmentation::UnicodeSegmentation;

    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut tokens = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;

    while index < graphemes.len() {
        let grapheme = graphemes[index];

        // :apelido: de emoji do servidor.
        if grapheme == ":"
            && let Some(end) = graphemes[index + 1..]
                .iter()
                .position(|part| *part == ":")
                .map(|offset| index + 1 + offset)
        {
            let name = graphemes[index + 1..end].concat();
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

        if grapheme.chars().any(is_emoji) {
            if !buffer.is_empty() {
                tokens.push(Token::Text(std::mem::take(&mut buffer)));
            }
            // Um grafema preserva ZWJ, tons, bandeiras e teclas combinadas,
            // sem juntar emojis vizinhos. O raster desenha um glifo por token.
            tokens.push(Token::Unicode(grapheme.to_owned()));
            index += 1;
            continue;
        }

        buffer.push_str(grapheme);
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

/// Abrevia a prévia sem cortar shortcodes ou sequências Unicode de emoji.
/// Cada imagem ocupa uma posição; o texto mantém os índices das menções.
pub fn elide_preview(text: &str, custom: &[crate::state::CustomEmoji], max: usize) -> String {
    let tokens = tokenize(text, custom);
    let length: usize = tokens.iter().map(|token| match token {
        Token::Text(chunk) => chunk.chars().count(),
        _ => 1,
    }).sum();
    if length <= max { return text.to_owned(); }
    if max == 0 { return String::new(); }
    let mut remaining = max - 1;
    let mut end = 0;
    for token in tokens {
        match token {
            Token::Text(chunk) => {
                for ch in chunk.chars().take(remaining) {
                    end += ch.len_utf8();
                    remaining -= 1;
                }
            }
            Token::Unicode(glyph) if remaining > 0 => {
                end += glyph.len();
                remaining -= 1;
            }
            Token::Custom(_) if remaining > 0 => {
                // O tokenizer já reconheceu :nome: neste ponto do texto.
                // Usar a fonte preserva o nome quando há aliases do mesmo id.
                end += text[end + 1..].find(':').expect("custom emoji has a closing colon") + 2;
                remaining -= 1;
            }
            _ => break,
        }
        if remaining == 0 { break; }
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_keep_custom_and_unicode_emojis_whole_at_the_limit() {
        let custom = [crate::state::CustomEmoji {
            id: "e-1".into(), name: "papo".into(), blob: None,
        }, crate::state::CustomEmoji {
            id: "e-1".into(), name: "outro_nome".into(), blob: None,
        }];
        for emoji in [":papo:", ":outro_nome:", "👩‍💻", "👍🏽", "🇧🇷", "1️⃣"] {
            let content = format!("{}{} fim", "á".repeat(78), emoji);
            assert_eq!(elide_preview(&content, &custom, 80), format!("{}{}…", "á".repeat(78), emoji));
            assert_eq!(elide_preview(&content, &custom, 79), format!("{}…", "á".repeat(78)));
        }
        assert_eq!(elide_preview(":papo:👩‍💻", &custom, 2), ":papo:👩‍💻");
        assert_eq!(elide_preview(":desconhecido:", &custom, 5), ":des…");
        assert_eq!(elide_preview("oi", &custom, 0), "");
        assert_eq!(elide_preview("oi", &custom, 1), "…");
    }

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
    fn emojis_vizinhos_sao_tokens_independentes() {
        let tokens = tokenize("😀🎉❤️", &[]);
        assert_eq!(
            tokens,
            vec![
                Token::Unicode("😀".into()),
                Token::Unicode("🎉".into()),
                Token::Unicode("❤️".into()),
            ]
        );
        assert!(jumbo(&tokens));
        assert!(!jumbo(&tokenize("😀🎉❤️👍", &[])));
    }

    #[test]
    fn preserva_grafemas_de_emoji_sem_juntar_vizinhos() {
        for parts in [
            vec!["👩‍💻", "👨‍👩‍👧‍👦"],
            vec!["👍🏽", "👍🏻"],
            vec!["🇧🇷", "🇵🇹"],
            vec!["1️⃣", "#️⃣", "*️⃣"],
            vec![
                "🏴\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
                "🎉",
            ],
        ] {
            let text = parts.concat();
            let expected: Vec<_> = parts
                .into_iter()
                .map(|part| Token::Unicode(part.into()))
                .collect();
            assert_eq!(tokenize(&text, &[]), expected, "{text}");
        }
        assert_eq!(
            tokenize("á 123#*", &[]),
            vec![Token::Text("á 123#*".into())]
        );
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
