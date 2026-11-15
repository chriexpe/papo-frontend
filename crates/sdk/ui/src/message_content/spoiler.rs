//! Inline `||spoiler||` resolution over `CommonMark` inline streams within depth limits.

use crate::message_content::model::{Inline, TextStyle};

/// Pre-merged inline piece keeping escaped `\|` pipes separate from unescaped `||` delimiters.
#[derive(Clone, Debug)]
pub(crate) enum InlinePiece {
    Text {
        text: String,
        style: TextStyle,
        link: Option<String>,
        allow_tokens: bool,
    },
    EscapedPipe {
        style: TextStyle,
        link: Option<String>,
        allow_tokens: bool,
    },
    /// Already resolved leaf (e.g. a mention, custom emoji, or non-token link/image fallback)
    /// that must never be split by `||` spoiler delimiters.
    Literal(Inline),
    Code(String),
    LineBreak,
}

enum Atom {
    Delim {
        style: TextStyle,
        link: Option<String>,
    },
    Inline(Inline),
}

struct Frame {
    delim_style: TextStyle,
    delim_link: Option<String>,
    items: Vec<Inline>,
}

/// Parses balanced `||...||` spoiler delimiters within `pieces`, honoring `max_depth`.
///
/// Returns `None` if nesting exceeds `max_depth` (`Err(())` would trigger literal fallback), or
/// `Ok(Vec<Inline>)` with adjacent same-style/same-link text leaves coalesced.
pub(crate) fn resolve_spoilers(pieces: Vec<InlinePiece>, max_depth: usize) -> Vec<Inline> {
    if !pieces.iter().any(|piece| {
        matches!(
            piece,
            InlinePiece::Text {
                text,
                allow_tokens: true,
                ..
            } if text.contains("||")
        )
    }) {
        return pieces_without_spoilers(pieces);
    }

    let atoms = split_delim_atoms(pieces);
    let mut stack = vec![Frame {
        delim_style: TextStyle::default(),
        delim_link: None,
        items: Vec::new(),
    }];

    for atom in atoms {
        match atom {
            Atom::Inline(inline) => {
                if let Some(top) = stack.last_mut() {
                    push_coalesced(&mut top.items, inline);
                }
            }
            Atom::Delim { style, link } => {
                let matching_frame = (1..stack.len())
                    .rev()
                    .find(|&idx| stack[idx].delim_link == link);
                if let Some(open_idx) = matching_frame {
                    if open_idx + 1 == stack.len()
                        && stack[open_idx].items.is_empty()
                        && let Some(unclosed) = stack.pop()
                    {
                        // Reject empty `||||` by restoring both delimiter pairs verbatim.
                        if let Some(parent) = stack.last_mut() {
                            push_text(parent, "||", unclosed.delim_style, unclosed.delim_link);
                            push_text(parent, "||", style, link);
                        }
                        continue;
                    }
                    // Unclosed inner spoilers inside `[open_idx + 1..]` stay literal text.
                    while stack.len() > open_idx + 1 {
                        if let Some(inner) = stack.pop()
                            && let Some(parent) = stack.last_mut()
                        {
                            push_text(parent, "||", inner.delim_style, inner.delim_link);
                            extend_coalesced(&mut parent.items, inner.items);
                        }
                    }
                    if let Some(closed) = stack.pop()
                        && let Some(parent) = stack.last_mut()
                    {
                        push_coalesced(&mut parent.items, Inline::Spoiler(closed.items));
                    }
                } else if stack.len() < max_depth {
                    // Opening pushes one more Spoiler container; leaf children stay <= max_depth.
                    stack.push(Frame {
                        delim_style: style,
                        delim_link: link,
                        items: Vec::new(),
                    });
                } else if let Some(top) = stack.last_mut() {
                    push_text(top, "||", style, link);
                }
            }
        }
    }

    while stack.len() > 1 {
        if let Some(unclosed) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            push_text(parent, "||", unclosed.delim_style, unclosed.delim_link);
            extend_coalesced(&mut parent.items, unclosed.items);
        }
    }

    let mut root = stack.pop().map_or_else(Vec::new, |frame| frame.items);
    root.retain(|inline| !matches!(inline, Inline::Text { text, .. } if text.is_empty()));
    root
}

fn pieces_without_spoilers(pieces: Vec<InlinePiece>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match piece {
            InlinePiece::Text {
                text,
                style,
                link,
                allow_tokens,
            } if !text.is_empty() => {
                push_coalesced(
                    &mut out,
                    Inline::Text {
                        text,
                        style,
                        link,
                        allow_tokens,
                    },
                );
            }
            InlinePiece::EscapedPipe {
                style,
                link,
                allow_tokens,
            } => push_coalesced(
                &mut out,
                Inline::Text {
                    text: String::from("|"),
                    style,
                    link,
                    allow_tokens,
                },
            ),
            InlinePiece::Literal(inline) => push_coalesced(&mut out, inline),
            InlinePiece::Code(code) => out.push(Inline::Code(code)),
            InlinePiece::LineBreak => out.push(Inline::LineBreak),
            InlinePiece::Text { .. } => {}
        }
    }
    out
}

fn split_delim_atoms(pieces: Vec<InlinePiece>) -> Vec<Atom> {
    let mut atoms = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match piece {
            InlinePiece::Text {
                text,
                style,
                link,
                allow_tokens: false,
            } => {
                if !text.is_empty() {
                    atoms.push(Atom::Inline(Inline::Text {
                        text,
                        style,
                        link,
                        allow_tokens: false,
                    }));
                }
            }
            InlinePiece::Text {
                mut text,
                style,
                link,
                allow_tokens: true,
            } => {
                if !text.contains("||") {
                    if !text.is_empty() {
                        atoms.push(Atom::Inline(Inline::Text {
                            text,
                            style,
                            link,
                            allow_tokens: true,
                        }));
                    }
                    continue;
                }
                let mut rest = text.as_str();
                while let Some(pos) = rest.find("||") {
                    if pos > 0 {
                        atoms.push(Atom::Inline(Inline::Text {
                            text: rest[..pos].to_owned(),
                            style,
                            link: link.clone(),
                            allow_tokens: true,
                        }));
                    }
                    atoms.push(Atom::Delim {
                        style,
                        link: link.clone(),
                    });
                    rest = &rest[pos + 2..];
                }
                if !rest.is_empty() {
                    atoms.push(Atom::Inline(Inline::Text {
                        text: if rest.len() == text.len() {
                            std::mem::take(&mut text)
                        } else {
                            rest.to_owned()
                        },
                        style,
                        link,
                        allow_tokens: true,
                    }));
                }
            }
            InlinePiece::EscapedPipe {
                style,
                link,
                allow_tokens,
            } => {
                atoms.push(Atom::Inline(Inline::Text {
                    text: String::from("|"),
                    style,
                    link,
                    allow_tokens,
                }));
            }
            InlinePiece::Literal(inline) => atoms.push(Atom::Inline(inline)),
            InlinePiece::Code(code) => atoms.push(Atom::Inline(Inline::Code(code))),
            InlinePiece::LineBreak => atoms.push(Atom::Inline(Inline::LineBreak)),
        }
    }
    atoms
}

fn push_text(frame: &mut Frame, text: &str, style: TextStyle, link: Option<String>) {
    push_coalesced(
        &mut frame.items,
        Inline::Text {
            text: text.to_owned(),
            style,
            link,
            allow_tokens: true,
        },
    );
}

fn extend_coalesced(target: &mut Vec<Inline>, items: Vec<Inline>) {
    for item in items {
        push_coalesced(target, item);
    }
}

pub(crate) fn push_coalesced(target: &mut Vec<Inline>, item: Inline) {
    match (target.last_mut(), item) {
        (
            Some(Inline::Text {
                text: existing,
                style: existing_style,
                link: existing_link,
                allow_tokens: existing_allow_tokens,
            }),
            Inline::Text {
                text: addition,
                style: addition_style,
                link: addition_link,
                allow_tokens: addition_allow_tokens,
            },
        ) if *existing_style == addition_style
            && *existing_link == addition_link
            && *existing_allow_tokens == addition_allow_tokens =>
        {
            existing.push_str(&addition);
        }
        (_, Inline::Text { text, .. }) if text.is_empty() => {}
        (_, other) => target.push(other),
    }
}
