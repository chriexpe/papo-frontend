//! The strings of a message's actions (`docs/design.md` §10): the hover
//! bar, the message and link menus, reactions, pins, and the notice that
//! says one of them did not happen. The catalog entries are `action.*`,
//! `menu.*`, `link_menu.*`, `reaction.*`, `reactions.*`, `pins.*` and
//! `notice.*`.

use super::{I18N, Key, PluralKey, catalog};

/// A reaction you added or removed did not happen.
pub(crate) fn notice_reaction_failed() -> &'static str {
    text!("notice.reaction_failed")
}

/// Pinning a message did not happen.
pub(crate) fn notice_pin_failed() -> &'static str {
    text!("notice.pin_failed")
}

/// Unpinning a message did not happen.
pub(crate) fn notice_unpin_failed() -> &'static str {
    text!("notice.unpin_failed")
}

/// A quick reaction's spoken name: "React with 👍".
pub(crate) fn reaction_react(emoji: &str) -> String {
    template!("reaction.react", "emoji" => emoji)
}

/// A quick reaction you already gave, which takes it back.
pub(crate) fn reaction_take_back(emoji: &str) -> String {
    template!("reaction.take_back", "emoji" => emoji)
}

/// The message menu's header: "Lia · Today at 14:02".
pub(crate) fn menu_header(author: &str, day: &str, time: &str) -> String {
    template!("menu.header", "author" => author, "day" => day, "time" => time)
}

/// Reply, in the message menu.
pub(crate) fn menu_reply() -> &'static str {
    text!("menu.reply")
}

/// Edit, in the message menu.
pub(crate) fn menu_edit() -> &'static str {
    text!("menu.edit")
}

/// Pin message.
pub(crate) fn menu_pin() -> &'static str {
    text!("menu.pin")
}

/// Unpin message.
pub(crate) fn menu_unpin() -> &'static str {
    text!("menu.unpin")
}

/// Copy text: the message as drawn.
pub(crate) fn menu_copy_text() -> &'static str {
    text!("menu.copy_text")
}

/// Copy as Markdown: the message's source.
pub(crate) fn menu_copy_markdown() -> &'static str {
    text!("menu.copy_markdown")
}

/// Copy ID.
pub(crate) fn menu_copy_id() -> &'static str {
    text!("menu.copy_id")
}

/// Delete message, last in the menu.
pub(crate) fn menu_delete() -> &'static str {
    text!("menu.delete")
}

/// Open link, in the message and link menus.
pub(crate) fn link_menu_open() -> &'static str {
    text!("link_menu.open")
}

/// Copy link, in the link menu.
pub(crate) fn link_menu_copy() -> &'static str {
    text!("link_menu.copy")
}

/// The Space key, as menus show it (a provider video's Play).
pub(crate) fn shortcut_space() -> &'static str {
    text!("shortcut.space")
}

/// Reply's key.
pub(crate) fn shortcut_reply() -> &'static str {
    text!("shortcut.reply")
}

/// Edit's key.
pub(crate) fn shortcut_edit() -> &'static str {
    text!("shortcut.edit")
}

/// Pin's key.
pub(crate) fn shortcut_pin() -> &'static str {
    text!("shortcut.pin")
}

/// Copy text's key.
pub(crate) fn shortcut_copy() -> &'static str {
    text!("shortcut.copy")
}

/// Delete's key.
pub(crate) fn shortcut_delete() -> &'static str {
    text!("shortcut.delete")
}

/// Add reaction, on the hover bar.
pub(crate) fn action_add_reaction() -> &'static str {
    text!("action.add_reaction")
}

/// Reply, on the hover bar.
pub(crate) fn action_reply() -> &'static str {
    text!("action.reply")
}

/// Edit, on the hover bar.
pub(crate) fn action_edit() -> &'static str {
    text!("action.edit")
}

/// More, on the hover bar: the message menu.
pub(crate) fn action_more() -> &'static str {
    text!("action.more")
}

/// The hover bar's spoken name.
pub(crate) fn action_bar() -> &'static str {
    text!("action.bar")
}

/// A chip's spoken name: "🧀, 4".
pub(crate) fn reaction_label(emoji: &str, count: u64) -> String {
    template!("reaction.label", "emoji" => emoji, "count" => count.to_string())
}

/// Your chip's spoken name: "🧀, 4, including you".
pub(crate) fn reaction_label_mine(emoji: &str, count: u64) -> String {
    template!("reaction.label_mine", "emoji" => emoji, "count" => count.to_string())
}

/// You, in who reacted: "You and Lia reacted with 🧀".
pub(crate) fn reaction_you() -> &'static str {
    text!("reaction.you")
}

/// Who reacted with `emoji`, from the first names `names` (you first) and
/// how many reacted in all, `count`.
pub(crate) fn reaction_who(names: &[&str], count: u64, emoji: &str) -> String {
    let listed = u64::try_from(names.len()).unwrap_or(u64::MAX);
    let others = count.saturating_sub(listed);
    match (names, others) {
        ([], _) => {
            const KEY: PluralKey = PluralKey::named(catalog::PLURALS, "reaction.who_count");
            I18N.plural_format(KEY, count, &[("emoji", &emoji)])
        }
        ([a], 0) => template!("reaction.who_one", "a" => a, "emoji" => emoji),
        ([a], _) => {
            const KEY: PluralKey = PluralKey::named(catalog::PLURALS, "reaction.who_one_more");
            I18N.plural_format(KEY, others, &[("a", a), ("emoji", &emoji)])
        }
        ([a, b], 0) => template!("reaction.who_two", "a" => a, "b" => b, "emoji" => emoji),
        ([a, b, ..], _) => {
            const KEY: PluralKey = PluralKey::named(catalog::PLURALS, "reaction.who_many");
            I18N.plural_format(KEY, others, &[("a", a), ("b", b), ("emoji", &emoji)])
        }
    }
}

/// View reactions, in the message menu.
pub(crate) fn menu_view_reactions() -> &'static str {
    text!("menu.view_reactions")
}

/// The reactions list's title.
pub(crate) fn reactions_title() -> &'static str {
    text!("reactions.title")
}

/// ← on the reactions list: back to the menu.
pub(crate) fn reactions_back() -> &'static str {
    text!("reactions.back")
}

/// The reactions list's tab of everyone.
pub(crate) fn reactions_all() -> &'static str {
    text!("reactions.all")
}

/// The mark beside your own row in the reactions list.
pub(crate) fn reactions_you() -> &'static str {
    text!("reactions.you")
}

/// An emoji's tab, spoken: "🧀, 4".
pub(crate) fn reactions_tab(emoji: &str, count: u64) -> String {
    template!("reactions.tab", "emoji" => emoji, "count" => count.to_string())
}

/// The list while a page of who reacted loads.
pub(crate) fn reactions_loading() -> &'static str {
    text!("reactions.loading")
}

/// The list when who reacted could not be loaded.
pub(crate) fn reactions_failed() -> &'static str {
    text!("reactions.failed")
}

/// The pins panel's title, and the pins button's name.
pub(crate) fn pins_title() -> &'static str {
    text!("pins.title")
}

/// The pins panel with nothing pinned.
pub(crate) fn pins_empty() -> &'static str {
    text!("pins.empty")
}

/// A pin's Unpin.
pub(crate) fn pins_unpin() -> &'static str {
    text!("pins.unpin")
}

/// A pin's spoken action: opening it in the conversation.
pub(crate) fn pins_jump() -> &'static str {
    text!("pins.jump")
}

/// The pins panel when the pins could not be read.
pub(crate) fn pins_failed() -> &'static str {
    text!("pins.failed")
}

/// The pins panel while the pins are read.
pub(crate) fn pins_loading() -> &'static str {
    text!("pins.loading")
}

/// The pins panel's key.
pub(crate) fn shortcut_pins() -> &'static str {
    text!("shortcut.pins")
}

/// The actions pill's spoken name.
pub(crate) fn header_actions() -> &'static str {
    text!("header.actions")
}

#[cfg(test)]
mod tests {
    use super::reaction_who;

    #[test]
    fn who_reacted_names_the_first_two_and_counts_the_rest() {
        assert_eq!(reaction_who(&[], 4, "🧀"), "4 people reacted with 🧀");
        assert_eq!(reaction_who(&[], 1, "🧀"), "1 person reacted with 🧀");
        assert_eq!(reaction_who(&["Lia"], 1, "🧀"), "Lia reacted with 🧀");
        assert_eq!(
            reaction_who(&["Lia"], 2, "🧀"),
            "Lia and 1 other reacted with 🧀"
        );
        assert_eq!(
            reaction_who(&["You", "Lia"], 2, "🧀"),
            "You and Lia reacted with 🧀"
        );
        assert_eq!(
            reaction_who(&["Lia", "Yuki"], 4, "🧀"),
            "Lia, Yuki and 2 others reacted with 🧀"
        );
    }
}
