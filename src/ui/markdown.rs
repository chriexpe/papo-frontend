//! Markdown das mensagens, sem egui: pré-processa o texto, interpreta com o
//! `pulldown-cmark` e devolve uma árvore pequena que `markdown_view` desenha.
//!
//! Segue o renderizador do cliente web (`src/lib/utils/markdown.ts`): CommonMark
//! mais tabelas, riscado e listas de tarefas, quebra de linha simples virando
//! linha nova, HTML cru descartado e só `http`, `https` e `mailto` viram link.
//! Duas diferenças de propósito: imagens não são carregadas (vira o texto
//! alternativo, ou o endereço, como link) e `@everyone` fica sem destaque.

use std::sync::Arc;

use papo_core::state::MentionBinding;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Parágrafo que só tem isto é um espaçador (linha em branco preservada).
const BLANK_LINE_MARKER: &str = "PAPOMESSAGEBLANKLINETOKEN";
/// Os sentinelas de menção usam o uso privado do Unicode: nenhum caractere
/// do markdown encosta neles, então um nome com `*` ou `_` não quebra a
/// interpretação.
const SENTINEL_OPEN: char = '\u{E000}';
const SENTINEL_CLOSE: char = '\u{E001}';
/// Quantas mensagens já interpretadas guardar antes de recomeçar o cache.
pub const CACHE_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    /// Texto corrido. Emoji e endereços soltos são achados na hora de
    /// desenhar. `link` é o destino quando o trecho está dentro de um link.
    Text {
        text: String,
        style: Style,
        link: Option<String>,
    },
    Code(String),
    Mention {
        user_id: String,
        label: String,
    },
    Break,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// `Some(marcada)` quando o item é uma tarefa (`- [x]`).
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Rule,
    Table {
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// Linha em branco extra, preservada como no cliente web.
    Spacer,
}

/// Quem foi mencionado, na ordem em que os sentinelas aparecem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub user_id: String,
    pub label: String,
}

/// Texto pronto para interpretar: menções já trocadas por sentinelas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prepared {
    pub source: String,
    pub mentions: Vec<Mention>,
}

impl Prepared {
    /// Chave do cache: o texto sozinho não basta, o sentinela `0` pode ser
    /// pessoas diferentes em mensagens diferentes.
    pub fn key(&self) -> String {
        let mut key = self.source.clone();
        for mention in &self.mentions {
            key.push('\u{E002}');
            key.push_str(&mention.user_id);
            key.push('\u{E002}');
            key.push_str(&mention.label);
        }
        key
    }
}

fn sentinel(index: usize) -> String {
    format!("{SENTINEL_OPEN}{index}{SENTINEL_CLOSE}")
}

/// Troca cada `@Nome` das ligações (e cada `@mention(<@id>)` que sobrou, de
/// quem o cliente ainda não conhece) por um sentinela, antes de interpretar.
pub fn substitute_mentions(display: &str, bindings: &[MentionBinding]) -> Prepared {
    // Caracteres do uso privado que já vinham no texto não podem se passar
    // por sentinela. As posições das ligações valem para o texto original,
    // então só dá para aplicá-las quando nada foi removido.
    let is_private = |c: char| matches!(c, SENTINEL_OPEN | SENTINEL_CLOSE | '\u{E002}');
    let chars: Vec<char> = display.chars().filter(|c| !is_private(*c)).collect();
    let untouched = chars.len() == display.chars().count();

    let mut spans: Vec<(usize, usize, Mention)> = Vec::new();
    if untouched {
        for binding in bindings {
            let len = 1 + binding.label.chars().count();
            if binding.start + len <= chars.len() && chars[binding.start] == '@' {
                spans.push((
                    binding.start,
                    len,
                    Mention {
                        user_id: binding.user_id.clone(),
                        label: binding.label.clone(),
                    },
                ));
            }
        }
    }
    spans.sort_by_key(|(start, _, _)| *start);
    let mut spans = spans.into_iter().peekable();

    let wire: Vec<char> = "@mention(<@".chars().collect();
    let mut out = String::with_capacity(display.len());
    let mut mentions: Vec<Mention> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Ligação sobreposta a outra já consumida: ignora.
        while spans.peek().is_some_and(|(start, _, _)| *start < i) {
            spans.next();
        }
        if spans.peek().is_some_and(|(start, _, _)| *start == i) {
            let (_, len, mention) = spans.next().expect("peeked");
            out.push_str(&sentinel(mentions.len()));
            mentions.push(mention);
            i += len;
            continue;
        }
        if chars[i..].starts_with(&wire) {
            let id_start = i + wire.len();
            let mut end = id_start;
            while end + 1 < chars.len() && !(chars[end] == '>' && chars[end + 1] == ')') {
                end += 1;
            }
            let id: String = chars[id_start..end].iter().collect();
            let closed = end + 1 < chars.len();
            if closed && id.len() >= 16 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
                out.push_str(&sentinel(mentions.len()));
                mentions.push(Mention {
                    user_id: id,
                    label: "usuário".to_owned(),
                });
                i = end + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    Prepared {
        source: out,
        mentions,
    }
}

// ---------------------------------------------------------------------------
// Pré-processamento do texto
// ---------------------------------------------------------------------------

/// Três crases sempre abrem um bloco de código, mesmo coladas no conteúdo
/// (```` ```{"ok":true}``` ````). Crase simples continua código em linha.
pub fn normalize_fenced_code_blocks(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + 16);
    let mut rest_start = 0;
    while let Some(open) = content[rest_start..].find("```").map(|i| i + rest_start) {
        let inner_start = open + 3;
        let Some(close) = content[inner_start..].find("```").map(|i| i + inner_start) else {
            break;
        };
        let inner = content[inner_start..close]
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let mut info = "";
        let mut body: &str = &inner;

        if let Some(stripped) = body.strip_prefix('\n') {
            body = stripped;
        } else if let Some(first_break) = body.find('\n') {
            let possible = body[..first_break].trim();
            if is_info_string(possible) {
                info = possible;
                body = &body[first_break + 1..];
            }
        }
        if let Some(stripped) = body.strip_suffix('\n') {
            body = stripped;
        }

        out.push_str(&content[rest_start..open]);
        let before_needs_break = open > 0 && content.as_bytes()[open - 1] != b'\n';
        let after = close + 3;
        let after_needs_break = after < content.len() && content.as_bytes()[after] != b'\n';
        if before_needs_break {
            out.push('\n');
        }
        out.push_str("```");
        out.push_str(info);
        out.push('\n');
        out.push_str(body);
        out.push_str("\n```");
        if after_needs_break {
            out.push('\n');
        }
        rest_start = after;
    }
    out.push_str(&content[rest_start..]);
    out
}

fn is_info_string(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'))
}

/// Cada linha em branco além da primeira vira um parágrafo-espaçador. Não
/// mexe dentro de blocos de código.
pub fn preserve_blank_lines(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    let mut in_fence = false;
    loop {
        let Some(at) = rest.find("```") else {
            push_segment(&mut out, rest, in_fence);
            break;
        };
        // A cerca entra no trecho de código; o texto "de fora" é o resto.
        if in_fence {
            out.push_str(&rest[..at + 3]);
        } else {
            push_segment(&mut out, &rest[..at], false);
            out.push_str("```");
        }
        in_fence = !in_fence;
        rest = &rest[at + 3..];
    }
    out
}

fn push_segment(out: &mut String, segment: &str, in_fence: bool) {
    if in_fence {
        out.push_str(segment);
        return;
    }
    let mut chars = segment.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\n' {
            out.push(c);
            continue;
        }
        let mut run = 1;
        while chars.peek() == Some(&'\n') {
            chars.next();
            run += 1;
        }
        if run < 2 {
            out.push('\n');
            continue;
        }
        out.push_str("\n\n");
        for index in 0..run - 1 {
            if index > 0 {
                out.push_str("\n\n");
            }
            out.push_str(BLANK_LINE_MARKER);
        }
        out.push_str("\n\n");
    }
}

/// Só `http`, `https` e `mailto` (e âncoras/caminhos) viram link; `javascript:`,
/// `data:`, `file:` e companhia ficam como texto.
pub fn is_safe_href(href: &str) -> bool {
    if href.is_empty() {
        return false;
    }
    if href.starts_with('#') || href.starts_with('/') {
        return true;
    }
    let Some(colon) = href.find(':') else {
        return false;
    };
    let scheme = &href[..colon];
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    valid
        && matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "mailto"
        )
}

/// Texto sem nenhuma sintaxe de markdown: quem chama pode seguir o caminho
/// simples, que desenha palavra por palavra como sempre foi.
pub fn is_plain(text: &str) -> bool {
    if text.chars().any(|c| {
        matches!(
            c,
            '*' | '_' | '~' | '`' | '\\' | '[' | ']' | '<' | '>' | '&' | '|' | '!' | '\t'
        )
    }) {
        return false;
    }
    text.split('\n').all(|line| {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() >= 4 {
            return false;
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        let ordered = digits > 0 && matches!(trimmed[digits..].chars().next(), Some('.' | ')'));
        !(ordered || trimmed.starts_with(['#', '-', '+', '=', '>']))
    })
}

// ---------------------------------------------------------------------------
// Interpretação
// ---------------------------------------------------------------------------

pub fn parse(prepared: &Prepared) -> Vec<Block> {
    let content = prepared.source.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalize_fenced_code_blocks(&content);
    let source = preserve_blank_lines(&normalized);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut builder = Builder {
        mentions: &prepared.mentions,
        bold: 0,
        italic: 0,
        strike: 0,
        links: Vec::new(),
        image_text: Vec::new(),
        task: None,
    };
    let mut events = Parser::new_ext(&source, options);
    builder.blocks(&mut events)
}

/// Atalho: o que `message_body` precisa, já em `Arc` para o cache.
pub fn parse_shared(prepared: &Prepared) -> Arc<Vec<Block>> {
    Arc::new(parse(prepared))
}

struct Builder<'a> {
    mentions: &'a [Mention],
    bold: u32,
    italic: u32,
    strike: u32,
    /// Destino de cada link aberto; `None` quando o esquema não é seguro.
    links: Vec<Option<String>>,
    /// Para cada imagem aberta: destino e se já saiu algum texto dela.
    image_text: Vec<(String, bool)>,
    task: Option<bool>,
}

impl Builder<'_> {
    fn style(&self) -> Style {
        Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
        }
    }

    fn link(&self) -> Option<String> {
        self.links.iter().rev().find_map(Clone::clone)
    }

    /// Lê eventos até o fim do contêiner atual (consumindo o `End`).
    fn blocks<'e>(&mut self, events: &mut impl Iterator<Item = Event<'e>>) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut pending: Vec<Inline> = Vec::new();
        while let Some(event) = events.next() {
            match event {
                Event::End(end) if !is_inline_end(&end) => break,
                Event::Start(Tag::Paragraph) => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    let inlines = self.inlines(events);
                    blocks.push(paragraph_or_spacer(inlines, self.mentions));
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    let inlines = self.inlines(events);
                    blocks.push(Block::Heading(level as u8, finish(inlines, self.mentions)));
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    let inner = self.blocks(events);
                    blocks.push(Block::Quote(inner));
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => {
                            let info = info.trim();
                            (!info.is_empty()).then(|| info.to_owned())
                        }
                        CodeBlockKind::Indented => None,
                    };
                    let mut text = String::new();
                    for event in events.by_ref() {
                        match event {
                            Event::Text(t) | Event::Code(t) => text.push_str(&t),
                            Event::End(_) => break,
                            _ => {}
                        }
                    }
                    if let Some(stripped) = text.strip_suffix('\n') {
                        text.truncate(stripped.len());
                    }
                    blocks.push(Block::Code {
                        lang,
                        text: restore_mentions(&text, self.mentions),
                    });
                }
                Event::Start(Tag::List(start)) => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    let mut items = Vec::new();
                    while let Some(event) = events.next() {
                        match event {
                            Event::Start(Tag::Item) => {
                                let outer = self.task.take();
                                let inner = self.blocks(events);
                                let task = self.task.take();
                                self.task = outer;
                                items.push(ListItem {
                                    task,
                                    blocks: inner,
                                });
                            }
                            Event::End(_) => break,
                            _ => {}
                        }
                    }
                    blocks.push(Block::List { start, items });
                }
                Event::Start(Tag::Table(_)) => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    blocks.push(self.table(events));
                }
                Event::Start(Tag::HtmlBlock) => {
                    // Descarta tudo até o fim do bloco.
                    for event in events.by_ref() {
                        if matches!(event, Event::End(TagEnd::HtmlBlock)) {
                            break;
                        }
                    }
                }
                Event::Rule => {
                    flush(&mut pending, &mut blocks, self.mentions);
                    blocks.push(Block::Rule);
                }
                // Item "apertado": o texto vem sem parágrafo em volta.
                other => {
                    if let Some(inline) = self.inline_event(other, events) {
                        pending.extend(inline);
                    }
                }
            }
        }
        flush(&mut pending, &mut blocks, self.mentions);
        blocks
    }

    fn table<'e>(&mut self, events: &mut impl Iterator<Item = Event<'e>>) -> Block {
        let mut head = Vec::new();
        let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
        let mut row: Vec<Vec<Inline>> = Vec::new();
        while let Some(event) = events.next() {
            match event {
                Event::Start(Tag::TableRow) => row = Vec::new(),
                Event::Start(Tag::TableCell) => {
                    let cell = self.inlines(events);
                    row.push(finish(cell, self.mentions));
                }
                Event::End(TagEnd::TableHead) => head = std::mem::take(&mut row),
                Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        Block::Table { head, rows }
    }

    /// Lê eventos de linha até o `End` do parágrafo/título/célula.
    fn inlines<'e>(&mut self, events: &mut impl Iterator<Item = Event<'e>>) -> Vec<Inline> {
        let mut out = Vec::new();
        while let Some(event) = events.next() {
            if matches!(&event, Event::End(end) if !is_inline_end(end)) {
                break;
            }
            if let Some(inline) = self.inline_event(event, events) {
                out.extend(inline);
            }
        }
        out
    }

    fn inline_event<'e>(
        &mut self,
        event: Event<'e>,
        _events: &mut impl Iterator<Item = Event<'e>>,
    ) -> Option<Vec<Inline>> {
        let mut out = Vec::new();
        match event {
            Event::Text(text) => {
                if let Some((_, any)) = self.image_text.last_mut() {
                    *any = true;
                }
                out.push(Inline::Text {
                    text: text.into_string(),
                    style: self.style(),
                    link: self.link(),
                });
            }
            Event::Code(code) => {
                if let Some((_, any)) = self.image_text.last_mut() {
                    *any = true;
                }
                out.push(Inline::Code(code.into_string()));
            }
            Event::SoftBreak | Event::HardBreak => out.push(Inline::Break),
            // HTML cru some: tags em linha e blocos inteiros.
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::TaskListMarker(checked) => self.task = Some(checked),
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => self.strike += 1,
            Event::End(TagEnd::Strikethrough) => self.strike = self.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let href = dest_url.into_string();
                self.links.push(is_safe_href(&href).then_some(href));
            }
            Event::End(TagEnd::Link) => {
                self.links.pop();
            }
            // Imagem remota é rastreamento: vira o texto alternativo, ou o
            // próprio endereço, como link.
            Event::Start(Tag::Image { dest_url, .. }) => {
                let href = dest_url.into_string();
                self.links.push(is_safe_href(&href).then(|| href.clone()));
                self.image_text.push((href, false));
            }
            Event::End(TagEnd::Image) => {
                if let Some((href, any)) = self.image_text.pop() {
                    if !any {
                        out.push(Inline::Text {
                            text: href,
                            style: self.style(),
                            link: self.link(),
                        });
                    }
                    self.links.pop();
                }
            }
            // Outros contêineres (notas, definições) não existem aqui.
            _ => return None,
        }
        Some(out)
    }
}

/// Fim de marcação em linha: não encerra o contêiner que está sendo lido.
fn is_inline_end(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image
    )
}

fn flush(pending: &mut Vec<Inline>, blocks: &mut Vec<Block>, mentions: &[Mention]) {
    if pending.is_empty() {
        return;
    }
    let inlines = finish(std::mem::take(pending), mentions);
    if !inlines.is_empty() {
        blocks.push(Block::Paragraph(inlines));
    }
}

fn paragraph_or_spacer(inlines: Vec<Inline>, mentions: &[Mention]) -> Block {
    if let [Inline::Text { text, .. }] = inlines.as_slice()
        && text == BLANK_LINE_MARKER
    {
        return Block::Spacer;
    }
    Block::Paragraph(finish(inlines, mentions))
}

/// Junta texto vizinho de mesmo estilo e troca os sentinelas por menções.
fn finish(inlines: Vec<Inline>, mentions: &[Mention]) -> Vec<Inline> {
    let mut merged: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match (merged.last_mut(), inline) {
            (
                Some(Inline::Text { text, style, link }),
                Inline::Text {
                    text: more,
                    style: more_style,
                    link: more_link,
                },
            ) if *style == more_style && *link == more_link => text.push_str(&more),
            (_, other) => merged.push(other),
        }
    }

    let mut out = Vec::with_capacity(merged.len());
    for inline in merged {
        match inline {
            Inline::Text { text, style, link } => {
                split_mentions(&text, style, link, mentions, &mut out)
            }
            Inline::Code(code) => out.push(Inline::Code(restore_mentions(&code, mentions))),
            other => out.push(other),
        }
    }
    out
}

fn split_mentions(
    text: &str,
    style: Style,
    link: Option<String>,
    mentions: &[Mention],
    out: &mut Vec<Inline>,
) {
    let mut rest = text;
    let mut plain = String::new();
    let push_plain = |plain: &mut String, out: &mut Vec<Inline>| {
        if !plain.is_empty() {
            out.push(Inline::Text {
                text: std::mem::take(plain),
                style,
                link: link.clone(),
            });
        }
    };
    while let Some(open) = rest.find(SENTINEL_OPEN) {
        plain.push_str(&rest[..open]);
        let after = &rest[open + SENTINEL_OPEN.len_utf8()..];
        let parsed = after.find(SENTINEL_CLOSE).and_then(|close| {
            let index = after[..close].parse::<usize>().ok()?;
            Some((mentions.get(index)?, close))
        });
        match parsed {
            Some((mention, close)) => {
                push_plain(&mut plain, out);
                out.push(Inline::Mention {
                    user_id: mention.user_id.clone(),
                    label: mention.label.clone(),
                });
                rest = &after[close + SENTINEL_CLOSE.len_utf8()..];
            }
            None => {
                // Sentinela quebrado: descarta só o caractere.
                rest = after;
            }
        }
    }
    plain.push_str(rest);
    push_plain(&mut plain, out);
}

/// Dentro de código a menção volta a ser `@Nome`, como texto.
fn restore_mentions(text: &str, mentions: &[Mention]) -> String {
    if !text.contains(SENTINEL_OPEN) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(SENTINEL_OPEN) {
        out.push_str(&rest[..open]);
        let after = &rest[open + SENTINEL_OPEN.len_utf8()..];
        let Some(close) = after.find(SENTINEL_CLOSE) else {
            rest = after;
            continue;
        };
        if let Some(mention) = after[..close]
            .parse::<usize>()
            .ok()
            .and_then(|index| mentions.get(index))
        {
            out.push('@');
            out.push_str(&mention.label);
        }
        rest = &after[close + SENTINEL_CLOSE.len_utf8()..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Testes: espelham `tests/markdown.test.ts` do cliente web
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(text: &str) -> Vec<Block> {
        parse(&Prepared {
            source: text.to_owned(),
            mentions: Vec::new(),
        })
    }

    /// Todo o texto visível, sem estilo, só para conferir o conteúdo.
    fn plain(blocks: &[Block]) -> String {
        fn inlines(list: &[Inline], out: &mut String) {
            for inline in list {
                match inline {
                    Inline::Text { text, .. } => out.push_str(text),
                    Inline::Code(code) => out.push_str(code),
                    Inline::Mention { label, .. } => {
                        out.push('@');
                        out.push_str(label);
                    }
                    Inline::Break => out.push('\n'),
                }
            }
        }
        fn walk(blocks: &[Block], out: &mut String) {
            for block in blocks {
                match block {
                    Block::Paragraph(list) | Block::Heading(_, list) => inlines(list, out),
                    Block::Quote(inner) => walk(inner, out),
                    Block::List { items, .. } => {
                        for item in items {
                            walk(&item.blocks, out);
                        }
                    }
                    Block::Code { text, .. } => out.push_str(text),
                    Block::Table { head, rows } => {
                        for cell in head.iter().chain(rows.iter().flatten()) {
                            inlines(cell, out);
                        }
                    }
                    Block::Rule | Block::Spacer => {}
                }
            }
        }
        let mut out = String::new();
        walk(blocks, &mut out);
        out
    }

    fn texts(inlines: &[Inline]) -> Vec<(&str, Style, Option<&str>)> {
        inlines
            .iter()
            .filter_map(|inline| match inline {
                Inline::Text { text, style, link } => {
                    Some((text.as_str(), *style, link.as_deref()))
                }
                _ => None,
            })
            .collect()
    }

    fn only_paragraph(blocks: &[Block]) -> &[Inline] {
        match blocks {
            [Block::Paragraph(inlines)] => inlines,
            other => panic!("esperava um parágrafo, veio {other:?}"),
        }
    }

    fn spacers(blocks: &[Block]) -> usize {
        blocks.iter().filter(|b| matches!(b, Block::Spacer)).count()
    }

    #[test]
    fn empty_content_is_empty() {
        assert!(blocks("").is_empty());
    }

    #[test]
    fn plain_text_is_one_paragraph() {
        let parsed = blocks("hello");
        assert_eq!(
            only_paragraph(&parsed),
            &[Inline::Text {
                text: "hello".into(),
                style: Style::default(),
                link: None
            }]
        );
    }

    #[test]
    fn raw_html_is_dropped() {
        assert!(blocks("<script>alert(1)</script>").is_empty());
        let parsed = blocks("a <b>bold</b> & c <d>e</d>");
        let text = plain(&parsed);
        assert!(!text.contains('<'));
        assert!(text.contains("a bold"));
        assert!(text.contains('&'));
        assert!(text.contains('e'));
    }

    #[test]
    fn bold_italic_and_strikethrough() {
        let parsed = blocks("**b** *i* ~~s~~");
        let runs = texts(only_paragraph(&parsed));
        assert!(runs.iter().any(|(t, s, _)| *t == "b" && s.bold));
        assert!(runs.iter().any(|(t, s, _)| *t == "i" && s.italic));
        assert!(runs.iter().any(|(t, s, _)| *t == "s" && s.strike));
    }

    #[test]
    fn underscore_italic_and_single_tilde_strike() {
        let parsed = blocks("_i_ ~s~");
        let runs = texts(only_paragraph(&parsed));
        assert!(runs.iter().any(|(t, s, _)| *t == "i" && s.italic));
        assert!(runs.iter().any(|(t, s, _)| *t == "s" && s.strike));
    }

    #[test]
    fn nested_styles_combine() {
        let parsed = blocks("***both***");
        let runs = texts(only_paragraph(&parsed));
        assert!(
            runs.iter()
                .any(|(t, s, _)| *t == "both" && s.bold && s.italic)
        );
    }

    #[test]
    fn single_newline_is_a_line_break() {
        let parsed = blocks("a\nb");
        assert!(only_paragraph(&parsed).contains(&Inline::Break));
    }

    #[test]
    fn blank_lines_between_paragraphs_are_preserved() {
        let parsed = blocks("a\n\nb");
        assert_eq!(spacers(&parsed), 1);
        assert_eq!(plain(&parsed), "ab");
        assert!(matches!(parsed[0], Block::Paragraph(_)));
        assert!(matches!(parsed[2], Block::Paragraph(_)));
    }

    #[test]
    fn consecutive_blank_lines_do_not_collapse() {
        assert_eq!(spacers(&blocks("a\n\n\nb")), 2);
        assert_eq!(spacers(&blocks("a\n\n\n\nb")), 3);
    }

    #[test]
    fn no_spacers_inside_fenced_code() {
        let parsed = blocks("```\na\n\nb\n```");
        assert_eq!(spacers(&parsed), 0);
        assert_eq!(
            parsed,
            vec![Block::Code {
                lang: None,
                text: "a\n\nb".into()
            }]
        );
    }

    #[test]
    fn fenced_code_keeps_its_content_literal() {
        let parsed = blocks("```\nfoo <bar> **x** :y:\n```");
        assert_eq!(
            parsed,
            vec![Block::Code {
                lang: None,
                text: "foo <bar> **x** :y:".into()
            }]
        );
    }

    #[test]
    fn triple_backticks_are_a_block_even_when_glued() {
        let one_line = blocks("```{\"ok\":true}```");
        assert_eq!(
            one_line,
            vec![Block::Code {
                lang: None,
                text: "{\"ok\":true}".into()
            }]
        );

        let attached_close = blocks("```\nfirst\nsecond```");
        assert_eq!(
            attached_close,
            vec![Block::Code {
                lang: None,
                text: "first\nsecond".into()
            }]
        );
    }

    #[test]
    fn fence_glued_to_surrounding_text_gets_its_own_lines() {
        let parsed = blocks("antes```x```depois");
        assert_eq!(parsed.len(), 3);
        assert!(matches!(parsed[0], Block::Paragraph(_)));
        assert_eq!(
            parsed[1],
            Block::Code {
                lang: None,
                text: "x".into()
            }
        );
        assert!(matches!(parsed[2], Block::Paragraph(_)));
    }

    #[test]
    fn fence_info_string_is_accepted_only_when_it_looks_like_one() {
        assert_eq!(
            normalize_fenced_code_blocks("```rust\nfn x() {}\n```"),
            "```rust\nfn x() {}\n```"
        );
        // Primeira linha com espaço: é conteúdo, não linguagem.
        assert_eq!(
            normalize_fenced_code_blocks("```not a lang\nbody```"),
            "```\nnot a lang\nbody\n```"
        );
        assert_eq!(
            normalize_fenced_code_blocks("```c++\nx```"),
            "```c++\nx\n```"
        );
        let parsed = blocks("```json\n{}\n```");
        assert_eq!(
            parsed,
            vec![Block::Code {
                lang: Some("json".into()),
                text: "{}".into()
            }]
        );
    }

    #[test]
    fn normalize_adds_newlines_around_glued_fences() {
        assert_eq!(
            normalize_fenced_code_blocks("a```b```c"),
            "a\n```\nb\n```\nc"
        );
        assert_eq!(normalize_fenced_code_blocks("```a```"), "```\na\n```");
        // Sem fecho, nada muda.
        assert_eq!(normalize_fenced_code_blocks("```aberta"), "```aberta");
        assert_eq!(normalize_fenced_code_blocks("sem cerca"), "sem cerca");
    }

    #[test]
    fn single_backticks_stay_inline_code() {
        let parsed = blocks("before `inline` after");
        let inlines = only_paragraph(&parsed);
        assert!(inlines.contains(&Inline::Code("inline".into())));
        assert_eq!(plain(&parsed), "before inline after");
    }

    #[test]
    fn emoji_shortcodes_stay_literal_in_code() {
        let parsed = blocks("`x :y:`");
        assert!(only_paragraph(&parsed).contains(&Inline::Code("x :y:".into())));
    }

    #[test]
    fn headings_lists_quote_rule_and_table() {
        let parsed = blocks("# T\n- a\n- b\n\n> q\n\n***\n\n| c | d |\n|--|--|\n| 1 | 2 |");
        assert!(matches!(&parsed[0], Block::Heading(1, _)));
        assert!(
            parsed
                .iter()
                .any(|b| matches!(b, Block::List { items, .. } if items.len() == 2))
        );
        assert!(parsed.iter().any(|b| matches!(b, Block::Quote(_))));
        assert!(parsed.contains(&Block::Rule));
        let table = parsed
            .iter()
            .find_map(|b| match b {
                Block::Table { head, rows } => Some((head, rows)),
                _ => None,
            })
            .expect("tabela");
        assert_eq!(table.0.len(), 2);
        assert_eq!(table.1.len(), 1);
        assert_eq!(plain(&[Block::Paragraph(table.0[0].clone())]), "c");
        assert_eq!(plain(&[Block::Paragraph(table.1[0][0].clone())]), "1");
    }

    #[test]
    fn heading_levels() {
        for level in 1..=6u8 {
            let parsed = blocks(&format!("{} t", "#".repeat(level as usize)));
            assert!(matches!(&parsed[0], Block::Heading(l, _) if *l == level));
        }
    }

    #[test]
    fn ordered_nested_and_task_lists() {
        let parsed = blocks("3. a\n4. b\n   - c\n\n- [x] feito\n- [ ] falta");
        let Block::List { start, items } = &parsed[0] else {
            panic!("lista");
        };
        assert_eq!(*start, Some(3));
        assert_eq!(items.len(), 2);
        assert!(
            items[1]
                .blocks
                .iter()
                .any(|b| matches!(b, Block::List { .. }))
        );
        let tasks = parsed
            .iter()
            .filter_map(|b| match b {
                Block::List { items, .. } => Some(items),
                _ => None,
            })
            .flatten()
            .filter_map(|item| item.task)
            .collect::<Vec<_>>();
        assert_eq!(tasks, vec![true, false]);
    }

    #[test]
    fn links_and_autolinks() {
        let parsed = blocks("[t](https://e.com/x) <https://e.com/y>");
        let runs = texts(only_paragraph(&parsed));
        assert!(
            runs.iter()
                .any(|(t, _, l)| *t == "t" && *l == Some("https://e.com/x"))
        );
        assert!(
            runs.iter()
                .any(|(t, _, l)| *t == "https://e.com/y" && *l == Some("https://e.com/y"))
        );
    }

    #[test]
    fn unsafe_link_schemes_render_as_plain_text() {
        let parsed = blocks(
            "[x](javascript:alert(1)) [y](vbscript:msg) [z](data:text/html,1) [w](file:///etc/passwd) [ok](https://e.com) [m](mailto:a@b.c)",
        );
        let runs = texts(only_paragraph(&parsed));
        // Os quatro viram texto comum e se juntam ao que vem em volta.
        assert!(
            runs.iter()
                .any(|(t, _, l)| t.starts_with("x y z w") && l.is_none())
        );
        assert!(runs.iter().all(|(t, _, l)| l.is_none() || !t.contains('x')));
        assert!(runs.iter().any(|(t, _, l)| *t == "ok" && l.is_some()));
        assert!(
            runs.iter()
                .any(|(t, _, l)| *t == "m" && *l == Some("mailto:a@b.c"))
        );
    }

    #[test]
    fn safe_href_rules() {
        assert!(is_safe_href("https://a.com"));
        assert!(is_safe_href("HTTP://a.com"));
        assert!(is_safe_href("mailto:a@b.c"));
        assert!(is_safe_href("#ancora"));
        assert!(is_safe_href("/caminho"));
        assert!(!is_safe_href(""));
        assert!(!is_safe_href("javascript:alert(1)"));
        assert!(!is_safe_href("data:text/html,x"));
        assert!(!is_safe_href("ftp://a.com"));
        assert!(!is_safe_href("sem-esquema.com"));
    }

    #[test]
    fn images_are_never_loaded() {
        let parsed = blocks("![gato](https://e.com/gato.png)");
        let runs = texts(only_paragraph(&parsed));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, "gato");
        assert_eq!(runs[0].2, Some("https://e.com/gato.png"));

        let parsed = blocks("![](https://e.com/gato.png)");
        let runs = texts(only_paragraph(&parsed));
        assert_eq!(runs[0].0, "https://e.com/gato.png");
        assert_eq!(runs[0].2, Some("https://e.com/gato.png"));

        // Esquema inseguro: só o texto, sem link.
        let parsed = blocks("![x](javascript:alert(1))");
        assert_eq!(texts(only_paragraph(&parsed))[0].2, None);
    }

    #[test]
    fn emoji_shortcodes_inside_formatting_and_tables_stay_in_the_text() {
        let parsed = blocks("**bold :x: text**\n\n| col |\n|--|\n| :x: |");
        let runs = texts(match &parsed[0] {
            Block::Paragraph(inlines) => inlines,
            other => panic!("{other:?}"),
        });
        assert!(runs.iter().any(|(t, s, _)| t.contains(":x:") && s.bold));
        let Some(Block::Table { rows, .. }) = parsed.last() else {
            panic!("tabela");
        };
        assert_eq!(plain(&[Block::Paragraph(rows[0][0].clone())]), ":x:");
    }

    #[test]
    fn numeric_character_references_are_decoded() {
        assert_eq!(plain(&blocks("&#65;")), "A");
    }

    #[test]
    fn backslash_escapes_are_literal() {
        let parsed = blocks("\\*not bold\\*");
        assert_eq!(plain(&parsed), "*not bold*");
        assert!(
            texts(only_paragraph(&parsed))
                .iter()
                .all(|(_, s, _)| !s.bold && !s.italic)
        );
    }

    #[test]
    fn spoiler_and_double_underscore_follow_the_web() {
        // `||x||` é literal e `__x__` é negrito, como no cliente web.
        assert_eq!(plain(&blocks("||x||")), "||x||");
        let parsed = blocks("__x__");
        assert!(
            texts(only_paragraph(&parsed))
                .iter()
                .any(|(t, s, _)| *t == "x" && s.bold)
        );
    }

    #[test]
    fn crlf_is_treated_as_lf() {
        assert_eq!(spacers(&blocks("a\r\n\r\nb")), 1);
    }

    // -- menções ------------------------------------------------------------

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    fn binding(start: usize, label: &str, id: &str) -> MentionBinding {
        MentionBinding {
            start,
            label: label.into(),
            user_id: id.into(),
        }
    }

    #[test]
    fn mention_becomes_a_pill_even_with_markdown_in_the_name() {
        let display = "oi @*Ana_ *x* tudo bem";
        let prepared = substitute_mentions(display, &[binding(3, "*Ana_ *x*", ID)]);
        assert!(!prepared.source.contains("Ana"));
        let parsed = parse(&prepared);
        let inlines = only_paragraph(&parsed);
        assert!(inlines.contains(&Inline::Mention {
            user_id: ID.into(),
            label: "*Ana_ *x*".into()
        }));
        // O resto da frase continua inteiro.
        assert_eq!(plain(&parsed), "oi @*Ana_ *x* tudo bem");
    }

    #[test]
    fn mention_inside_bold_and_headings() {
        let prepared = substitute_mentions("**@Ana**", &[binding(2, "Ana", ID)]);
        let parsed = parse(&prepared);
        assert!(only_paragraph(&parsed).contains(&Inline::Mention {
            user_id: ID.into(),
            label: "Ana".into()
        }));
        let prepared = substitute_mentions("# oi @Ana", &[binding(5, "Ana", ID)]);
        assert!(matches!(&parse(&prepared)[0], Block::Heading(1, inl)
            if inl.iter().any(|i| matches!(i, Inline::Mention { .. }))));
    }

    #[test]
    fn mention_in_code_is_literal_text() {
        let prepared = substitute_mentions("`@Ana`", &[binding(1, "Ana", ID)]);
        let parsed = parse(&prepared);
        assert!(only_paragraph(&parsed).contains(&Inline::Code("@Ana".into())));
        let prepared = substitute_mentions("```\n@Ana\n```", &[binding(4, "Ana", ID)]);
        assert_eq!(
            parse(&prepared),
            vec![Block::Code {
                lang: None,
                text: "@Ana".into()
            }]
        );
    }

    #[test]
    fn several_mentions_keep_their_own_ids() {
        let other = "99999999-8888-7777-6666-555555555555";
        let prepared = substitute_mentions(
            "@Ana e @Ana",
            &[binding(0, "Ana", ID), binding(7, "Ana", other)],
        );
        let parsed = parse(&prepared);
        let ids: Vec<_> = only_paragraph(&parsed)
            .iter()
            .filter_map(|i| match i {
                Inline::Mention { user_id, .. } => Some(user_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![ID, other]);
    }

    #[test]
    fn unknown_wire_mention_still_becomes_a_pill() {
        let text = format!("oi @mention(<@{ID}>)!");
        let prepared = substitute_mentions(&text, &[]);
        let parsed = parse(&prepared);
        assert!(only_paragraph(&parsed).contains(&Inline::Mention {
            user_id: ID.into(),
            label: "usuário".into()
        }));
        // Id curto demais não é menção.
        let prepared = substitute_mentions("@mention(<@abc>)", &[]);
        assert!(prepared.mentions.is_empty());
    }

    #[test]
    fn everyone_stays_plain() {
        let parsed = blocks("@everyone oi");
        assert_eq!(plain(&parsed), "@everyone oi");
        assert!(
            !only_paragraph(&parsed)
                .iter()
                .any(|i| matches!(i, Inline::Mention { .. }))
        );
    }

    #[test]
    fn private_use_characters_in_the_text_cannot_fake_a_mention() {
        let prepared = substitute_mentions("a\u{E000}0\u{E001}b", &[]);
        assert!(prepared.mentions.is_empty());
        assert!(!prepared.source.contains(SENTINEL_OPEN));
    }

    #[test]
    fn cache_key_depends_on_who_was_mentioned() {
        let a = substitute_mentions("@Ana", &[binding(0, "Ana", ID)]);
        let b = substitute_mentions(
            "@Bia",
            &[binding(0, "Bia", "99999999-8888-7777-6666-555555555555")],
        );
        assert_eq!(a.source, b.source);
        assert_ne!(a.key(), b.key());
    }

    // -- caminho simples ------------------------------------------------------

    #[test]
    fn plain_detection() {
        for text in [
            "oi tudo bem?",
            "10-20 e 3.5",
            "a\nb\n\nc",
            "ola :x: 👋",
            "https://a.com/x?y=1",
        ] {
            assert!(is_plain(text), "{text:?} devia ser simples");
        }
        for text in [
            "**b**",
            "_i_",
            "~s~",
            "`c`",
            "\\*",
            "[a](b)",
            "# t",
            "- a",
            "1. a",
            "> q",
            "| a |",
            "a\n- b",
            "    codigo",
            "ola<b>",
            "&amp;",
            "![a](b)",
            "---",
            "===",
        ] {
            assert!(!is_plain(text), "{text:?} devia ir pelo markdown");
        }
    }
}
