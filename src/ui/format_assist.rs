//! Autocompletar do formato de texto (` ``` `, `**`, `||`...), no estilo do
//! Discord: ao abrir um marcador que ainda não foi fechado, o composer
//! oferece o fechamento como texto fantasma no fim da linha. Tab, seta
//! para a direita ou um toque nele inserem o fechamento.
//!
//! Aqui só há lógica de texto, sem egui, para poder ser testada sozinha.

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

/// O marcador em aberto mais interno antes do cursor, se houver.
fn open_marker(prefix: &[char]) -> Option<Marker> {
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
    stack.pop()
}

/// O fechamento a oferecer para `text` com o cursor em `caret` (em
/// caracteres), ou `None` se não houver o que fechar.
///
/// Só oferece quando o cursor está no fim do texto ou da linha: no meio de
/// uma frase o usuário está editando, não escrevendo.
pub fn pending_closer(text: &str, caret: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let caret = caret.min(chars.len());
    if chars[caret..]
        .iter()
        .take_while(|c| **c != '\n')
        .any(|c| !c.is_whitespace())
    {
        return None;
    }
    let marker = open_marker(&chars[..caret])?;
    Some(match marker {
        // A cerca fecha em linha própria.
        Marker::Fence if caret > 0 && chars[caret - 1] != '\n' => "\n```".to_owned(),
        other => other.token().to_owned(),
    })
}

/// Insere o fechamento no cursor e devolve a nova posição do cursor.
pub fn accept(text: &mut String, caret: usize) -> Option<usize> {
    let closer = pending_closer(text, caret)?;
    let at = text
        .char_indices()
        .nth(caret)
        .map_or(text.len(), |(byte, _)| byte);
    text.insert_str(at, &closer);
    Some(caret + closer.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closer(text: &str) -> Option<String> {
        pending_closer(text, text.chars().count())
    }

    #[test]
    fn plain_text_has_nothing_to_close() {
        assert_eq!(closer("oi pessoal"), None);
        assert_eq!(closer(""), None);
    }

    #[test]
    fn opens_and_closes_inline_markers() {
        assert_eq!(closer("**negrito").as_deref(), Some("**"));
        assert_eq!(closer("**negrito**"), None);
        assert_eq!(closer("veja `codigo").as_deref(), Some("`"));
        assert_eq!(closer("veja `codigo`"), None);
        assert_eq!(closer("||segredo").as_deref(), Some("||"));
        assert_eq!(closer("~~riscado").as_deref(), Some("~~"));
        assert_eq!(closer("__sub").as_deref(), Some("__"));
        assert_eq!(closer("*it").as_deref(), Some("*"));
    }

    #[test]
    fn nested_closes_the_innermost_first() {
        assert_eq!(closer("**a ||b").as_deref(), Some("||"));
        assert_eq!(closer("**a ||b||").as_deref(), Some("**"));
    }

    #[test]
    fn fence_closes_on_its_own_line() {
        assert_eq!(closer("```").as_deref(), Some("\n```"));
        assert_eq!(closer("```rust\nfn x() {}").as_deref(), Some("\n```"));
        assert_eq!(closer("```rust\nfn x() {}\n").as_deref(), Some("```"));
        assert_eq!(closer("```a```"), None);
    }

    #[test]
    fn markers_inside_code_are_ignored() {
        assert_eq!(closer("```\n**x"), Some("\n```".into()));
        assert_eq!(closer("`**x"), Some("`".into()));
    }

    #[test]
    fn words_and_spaces_do_not_open() {
        assert_eq!(closer("snake_case_name"), None);
        assert_eq!(closer("2*3"), None);
        assert_eq!(closer("** solto"), None);
    }

    #[test]
    fn inline_markers_stop_at_newlines() {
        assert_eq!(closer("**a\nb"), None);
    }

    #[test]
    fn silent_when_caret_is_mid_sentence() {
        assert_eq!(pending_closer("**ab cd", 4), None);
        assert_eq!(pending_closer("**ab\nresto", 4).as_deref(), Some("**"));
    }

    #[test]
    fn accept_inserts_at_caret_with_unicode() {
        let mut text = String::from("**olá é");
        let caret = text.chars().count();
        assert_eq!(accept(&mut text, caret), Some(caret + 2));
        assert_eq!(text, "**olá é**");
        assert_eq!(accept(&mut text, caret + 2), None);
    }
}
