//! Fechamento automático do formato de texto (` ``` `, `` ` ``, `**`, `__`,
//! `~~`, `||`): ao completar um marcador, o fechamento já é escrito logo
//! depois do cursor, que fica entre os dois. Para sair, basta andar para a
//! direita: seta, Tab, um toque depois do texto ou segurar o espaço do
//! teclado do celular. Digitar o próprio fechamento por cima também o
//! atravessa, e apagar o marcador vazio leva o fechamento junto.
//!
//! O texto é de verdade (não um fantasma desenhado), então vale igual no
//! campo nativo do Android. Aqui só há lógica de texto, sem egui, para poder
//! ser testada sozinha.

/// Marcadores que o composer entende.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Fence,
    Code,
    Bold,
    Underline,
    Strike,
    Spoiler,
    Italic,
    ItalicUnderscore,
}

impl Marker {
    fn token(self) -> &'static str {
        match self {
            Marker::Fence => "```",
            Marker::Code => "`",
            Marker::Bold => "**",
            Marker::Underline => "__",
            Marker::Strike => "~~",
            Marker::Spoiler => "||",
            Marker::Italic => "*",
            Marker::ItalicUnderscore => "_",
        }
    }

    /// Do mais longo para o mais curto: `**` precisa ser tentado antes de `*`.
    const ALL: [Marker; 8] = [
        Marker::Fence,
        Marker::Bold,
        Marker::Underline,
        Marker::Strike,
        Marker::Spoiler,
        Marker::Code,
        Marker::Italic,
        Marker::ItalicUnderscore,
    ];
}

fn starts_with_at(chars: &[char], at: usize, token: &str) -> bool {
    token
        .chars()
        .enumerate()
        .all(|(offset, c)| chars.get(at + offset) == Some(&c))
}

/// A pilha de marcadores em aberto antes do cursor.
fn open_stack(prefix: &[char]) -> Vec<Marker> {
    let mut stack: Vec<Marker> = Vec::new();
    let mut i = 0;
    while i < prefix.len() {
        let c = prefix[i];
        // Cerca de código: só ela própria fecha, e atravessa linhas.
        if stack.last() == Some(&Marker::Fence) {
            if starts_with_at(prefix, i, "```") {
                stack.pop();
                i += 3;
            } else {
                i += 1;
            }
            continue;
        }
        // Marcadores de linha não atravessam quebras de linha.
        if c == '\n' {
            stack.retain(|marker| *marker == Marker::Fence);
            i += 1;
            continue;
        }
        // Dentro de código em linha só a crase fecha.
        if stack.last() == Some(&Marker::Code) {
            if c == '`' {
                stack.pop();
            }
            i += 1;
            continue;
        }

        let found = Marker::ALL
            .into_iter()
            .find(|marker| starts_with_at(prefix, i, marker.token()));
        let Some(marker) = found else {
            i += 1;
            continue;
        };
        let len = marker.token().chars().count();
        let before = i.checked_sub(1).map(|at| prefix[at]);
        let after = prefix.get(i + len).copied();

        let closes = stack.last() == Some(&marker) && before.is_some_and(|c| !c.is_whitespace());
        if closes {
            stack.pop();
        } else {
            // Abre se não estiver colado no meio de uma palavra (snake_case,
            // 2*3) e o que vem depois não for espaço. A cerca abre sempre.
            let single = matches!(marker, Marker::Italic | Marker::ItalicUnderscore);
            let boundary = before.is_none_or(|c| !c.is_alphanumeric());
            let tight = after.is_none_or(|c| !c.is_whitespace());
            let opens = match marker {
                Marker::Fence => true,
                _ if single => boundary && tight,
                _ => tight,
            };
            if opens {
                stack.push(marker);
            }
        }
        i += len;
    }
    stack
}

/// Quais marcadores fecham sozinhos. `*` e `_` soltos ficam de fora: são
/// o começo de `**`/`__`, aparecem em listas e em nomes como `snake_case`.
fn auto_closes(marker: Marker) -> bool {
    matches!(
        marker,
        Marker::Fence
            | Marker::Code
            | Marker::Bold
            | Marker::Underline
            | Marker::Strike
            | Marker::Spoiler
    )
}

/// Um fechamento escrito automaticamente que ainda está esperando o usuário
/// passar por ele.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    opener: Vec<char>,
    closer: Vec<char>,
    /// Índice (em caracteres) onde o marcador de abertura começa.
    start: usize,
    /// Índice (em caracteres) onde o fechamento começa.
    at: usize,
}

impl Pairing {
    fn opener_start(&self) -> usize {
        self.start
    }

    /// Nada digitado entre o marcador e o fechamento ainda.
    fn is_empty(&self) -> bool {
        self.at == self.start + self.opener.len()
    }

    /// A cerca de código fecha em linha própria e não ganha o espaço depois.
    fn is_fence(&self) -> bool {
        self.closer.first() == Some(&'\n')
    }
}

/// Fim de um par: o cursor fica depois do fechamento e, se o que vem em
/// seguida não for um espaço, um é escrito para o texto seguir solto.
fn leave_pair(chars: &mut Vec<char>, caret: usize) -> usize {
    if chars.get(caret).is_none_or(|c| !c.is_whitespace()) {
        chars.insert(caret.min(chars.len()), ' ');
    }
    caret + 1
}

/// O texto e o cursor a aplicar no lugar do que o usuário acabou de digitar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub caret: usize,
}

enum Change {
    Insert { index: usize, ch: char },
    Delete { index: usize },
    Other,
}

/// O que mudou de `prev` para `text`, usando o cursor para desfazer a
/// ambiguidade de caracteres repetidos (`**`).
fn classify(prev: &[char], text: &[char], caret: usize) -> Change {
    if text.len() == prev.len() + 1
        && caret >= 1
        && caret <= text.len()
        && prev[..caret - 1] == text[..caret - 1]
        && prev[caret - 1..] == text[caret..]
    {
        return Change::Insert {
            index: caret - 1,
            ch: text[caret - 1],
        };
    }
    if prev.len() == text.len() + 1
        && caret <= text.len()
        && prev[..caret] == text[..caret]
        && prev[caret + 1..] == text[caret..]
    {
        return Change::Delete { index: caret };
    }
    Change::Other
}

fn pairing_intact(text: &[char], pairing: &Pairing) -> bool {
    text.get(pairing.at..pairing.at + pairing.closer.len()) == Some(pairing.closer.as_slice())
}

/// Chamado quando o texto do composer mudou de `prev` para `text`, com o
/// cursor em `caret`. Devolve a edição a aplicar, se houver.
///
/// `pairing` guarda o fechamento automático em espera; esta função o mantém
/// em dia (desloca quando se digita antes dele, descarta quando ele some).
pub fn on_change(
    prev: &str,
    text: &str,
    caret: usize,
    pairing: &mut Option<Pairing>,
) -> Option<Edit> {
    if prev == text {
        return None;
    }
    let prev_chars: Vec<char> = prev.chars().collect();
    let chars: Vec<char> = text.chars().collect();
    let caret = caret.min(chars.len());
    let change = classify(&prev_chars, &chars, caret);

    if let Some(active) = pairing.clone() {
        match change {
            // Digitou o próprio fechamento: atravessa em vez de duplicar.
            Change::Insert { index, ch }
                if index == active.at
                    && Some(&ch) == active.closer.first()
                    && !(active.is_fence() && ch == '\n') =>
            {
                let mut rest = active;
                let inline = !rest.is_fence() && !rest.is_empty();
                rest.closer.remove(0);
                rest.at += 1;
                let moved = rest.at;
                let done = rest.closer.is_empty();
                *pairing = (!done).then_some(rest);
                let mut written = prev_chars.clone();
                let caret = if done && inline {
                    leave_pair(&mut written, moved)
                } else {
                    moved
                };
                return Some(Edit {
                    text: written.into_iter().collect(),
                    caret,
                });
            }
            // Enter (ou Shift+Enter) com o cursor dentro do par: o fechamento
            // é confirmado antes e a quebra de linha vem depois dele. Dentro
            // de uma cerca de código a linha nova é do código.
            Change::Insert { index, ch: '\n' }
                if !active.is_fence()
                    && index >= active.opener_start()
                    && index <= active.at =>
            {
                let end = active.at + active.closer.len();
                let mut written = prev_chars.clone();
                written.insert(end.min(written.len()), '\n');
                *pairing = None;
                return Some(Edit {
                    text: written.into_iter().collect(),
                    caret: end + 1,
                });
            }
            // Apagou o último caractere do marcador sem nada dentro: o
            // fechamento não tem mais motivo para ficar.
            Change::Delete { index }
                if active.is_empty()
                    && index >= active.start
                    && index < active.start + active.opener.len() =>
            {
                let mut cleaned = chars.clone();
                let at = active.at - 1;
                let end = (at + active.closer.len()).min(cleaned.len());
                cleaned.drain(at..end);
                *pairing = None;
                return Some(Edit {
                    text: cleaned.into_iter().collect(),
                    caret,
                });
            }
            Change::Insert { index, .. } if index <= active.at => {
                let mut shifted = active;
                if index < shifted.start {
                    shifted.start += 1;
                }
                shifted.at += 1;
                *pairing = pairing_intact(&chars, &shifted).then_some(shifted);
            }
            Change::Delete { index } if index < active.at => {
                let mut shifted = active;
                if index < shifted.start {
                    shifted.start = shifted.start.saturating_sub(1);
                }
                shifted.at -= 1;
                *pairing = pairing_intact(&chars, &shifted).then_some(shifted);
            }
            Change::Insert { .. } | Change::Delete { .. } => {
                *pairing = pairing_intact(&chars, &active).then_some(active);
            }
            Change::Other => {
                *pairing = pairing_intact(&chars, &active).then_some(active);
            }
        }
    }

    // Um marcador acabou de ser aberto?
    let Change::Insert { index, .. } = change else {
        return None;
    };
    let before = open_stack(&chars[..index]);
    let after = open_stack(&chars[..caret]);
    // `**` aparece trocando o `*` que já estava aberto, sem crescer a pilha.
    let marker = *after.last().filter(|marker| {
        (after.len() > before.len() || before.last() != Some(*marker))
            && auto_closes(**marker)
            && chars[..caret].ends_with(&marker.token().chars().collect::<Vec<_>>())
    })?;

    // Só no fim da linha: no meio de uma frase o usuário está editando.
    let rest = &chars[caret..];
    if rest
        .iter()
        .take_while(|c| **c != '\n')
        .any(|c| !c.is_whitespace())
    {
        return None;
    }
    let token: Vec<char> = marker.token().chars().collect();
    if rest.starts_with(&token) {
        return None;
    }
    let closer: Vec<char> = match marker {
        // A cerca fecha em linha própria.
        Marker::Fence => "\n```".chars().collect(),
        _ => token.clone(),
    };
    let mut written = chars;
    for (offset, c) in closer.iter().enumerate() {
        written.insert(caret + offset, *c);
    }
    *pairing = Some(Pairing {
        start: caret - token.len(),
        opener: token,
        closer,
        at: caret,
    });
    Some(Edit {
        text: written.into_iter().collect(),
        caret,
    })
}

/// Tab: se o cursor está dentro de um par automático, pula para depois do
/// fechamento (e deixa um espaço depois dele). Devolve a edição a aplicar.
pub fn skip_closer(text: &str, caret: usize, pairing: &mut Option<Pairing>) -> Option<Edit> {
    let active = pairing.clone()?;
    let mut chars: Vec<char> = text.chars().collect();
    if !pairing_intact(&chars, &active) {
        *pairing = None;
        return None;
    }
    if caret < active.opener_start() || caret > active.at {
        return None;
    }
    *pairing = None;
    let end = active.at + active.closer.len();
    let caret = if active.is_fence() || active.is_empty() {
        end
    } else {
        leave_pair(&mut chars, end)
    };
    Some(Edit {
        text: chars.into_iter().collect(),
        caret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simula o TextEdit: insere `c` no cursor e roda `on_change`.
    fn typing(text: &mut String, caret: &mut usize, pairing: &mut Option<Pairing>, input: &str) {
        for c in input.chars() {
            let prev = text.clone();
            let mut chars: Vec<char> = prev.chars().collect();
            chars.insert(*caret, c);
            *caret += 1;
            *text = chars.into_iter().collect();
            if let Some(edit) = on_change(&prev, text, *caret, pairing) {
                *text = edit.text;
                *caret = edit.caret;
            }
        }
    }

    fn backspace(text: &mut String, caret: &mut usize, pairing: &mut Option<Pairing>) {
        let prev = text.clone();
        let mut chars: Vec<char> = prev.chars().collect();
        *caret -= 1;
        chars.remove(*caret);
        *text = chars.into_iter().collect();
        if let Some(edit) = on_change(&prev, text, *caret, pairing) {
            *text = edit.text;
            *caret = edit.caret;
        }
    }

    fn session() -> (String, usize, Option<Pairing>) {
        (String::new(), 0, None)
    }

    #[test]
    fn bold_closes_itself_and_the_caret_stays_inside() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "**");
        assert_eq!((t.as_str(), c), ("****", 2));
        typing(&mut t, &mut c, &mut p, "oi");
        assert_eq!((t.as_str(), c), ("**oi**", 4));
    }

    #[test]
    fn typing_the_closer_walks_over_it() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "**oi**");
        // Terminou o par: um espaço para continuar fora do negrito.
        assert_eq!((t.as_str(), c), ("**oi** ", 7));
        assert!(p.is_none());
    }

    #[test]
    fn every_pair_marker_closes() {
        for (opener, expect) in [("`", "``"), ("__", "____"), ("~~", "~~~~"), ("||", "||||")] {
            let (mut t, mut c, mut p) = session();
            typing(&mut t, &mut c, &mut p, opener);
            assert_eq!(t, expect, "{opener}");
            assert_eq!(c, opener.chars().count());
        }
    }

    #[test]
    fn single_star_and_underscore_do_not_pair() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "snake_case * item");
        assert_eq!(t, "snake_case * item");
        assert!(p.is_none());
    }

    #[test]
    fn fence_closes_on_its_own_line() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "```");
        assert_eq!((t.as_str(), c), ("```\n```", 3));
    }

    #[test]
    fn third_backtick_upgrades_inline_code_to_a_fence() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "```");
        assert_eq!(t, "```\n```");
    }

    #[test]
    fn backspace_on_an_empty_pair_removes_the_closer_too() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "**");
        backspace(&mut t, &mut c, &mut p);
        assert_eq!((t.as_str(), c), ("*", 1));
        assert!(p.is_none());
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "`");
        backspace(&mut t, &mut c, &mut p);
        assert_eq!((t.as_str(), c), ("", 0));
    }

    #[test]
    fn backspace_inside_content_keeps_the_closer() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "**ab");
        backspace(&mut t, &mut c, &mut p);
        assert_eq!((t.as_str(), c), ("**a**", 3));
        assert!(p.is_some());
    }

    #[test]
    fn tab_jumps_past_the_closer_once() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "||segredo");
        assert_eq!(t, "||segredo||");
        let edit = skip_closer(&t, c, &mut p).unwrap();
        assert_eq!((edit.text.as_str(), edit.caret), ("||segredo|| ", 12));
        assert_eq!(skip_closer(&edit.text, 12, &mut p), None);
    }

    #[test]
    fn no_pairing_mid_sentence_or_when_already_closed() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "ab cd");
        c = 2;
        typing(&mut t, &mut c, &mut p, "`");
        assert_eq!(t, "ab` cd");
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "x ");
        t.push_str("**");
        c = 2;
        typing(&mut t, &mut c, &mut p, "**");
        assert_eq!(t, "x ****");
    }

    #[test]
    fn typing_before_the_closer_shifts_it() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "**ab");
        c = 3;
        typing(&mut t, &mut c, &mut p, "x");
        assert_eq!(t, "**axb**");
        let edit = skip_closer(&t, c, &mut p).unwrap();
        assert_eq!((edit.text.as_str(), edit.caret), ("**axb** ", 8));
    }

    #[test]
    fn unicode_is_counted_in_characters() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "á**é");
        assert_eq!((t.as_str(), c), ("á**é**", 4));
    }

    #[test]
    fn enter_inside_a_pair_commits_the_closer_first() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "**test");
        assert_eq!(t, "**test**");
        typing(&mut t, &mut c, &mut p, "\n");
        assert_eq!((t.as_str(), c), ("**test**\n", 9));
        assert!(p.is_none());
    }

    #[test]
    fn enter_inside_a_fence_stays_inside_the_code() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "```");
        typing(&mut t, &mut c, &mut p, "\n");
        // Abre uma linha para o código; a cerca continua em linha própria.
        assert_eq!((t.as_str(), c), ("```\n\n```", 4));
        typing(&mut t, &mut c, &mut p, "x");
        assert_eq!(t, "```\nx\n```");
    }

    #[test]
    fn a_fence_gets_no_trailing_space() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "```x");
        let edit = skip_closer(&t, c, &mut p).unwrap();
        assert_eq!(edit.text, "```x\n```");
    }

    #[test]
    fn no_extra_space_when_one_already_follows() {
        let (mut t, mut c, mut p) = session();
        typing(&mut t, &mut c, &mut p, "x ");
        c = 0;
        t = "**a** b".into();
        p = Some(Pairing {
            opener: vec!['*', '*'],
            closer: vec!['*', '*'],
            start: 0,
            at: 3,
        });
        c += 3;
        let edit = skip_closer(&t, c, &mut p).unwrap();
        assert_eq!((edit.text.as_str(), edit.caret), ("**a** b", 6));
    }
}
