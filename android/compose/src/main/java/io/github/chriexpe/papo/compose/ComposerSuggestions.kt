package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.EmojiView

internal sealed interface ComposerSuggestion {
    val key: String

    data object Everyone : ComposerSuggestion {
        override val key = "everyone"
    }

    data class User(val person: Person) : ComposerSuggestion {
        override val key = "person/${person.id}"
    }

    data class Sticker(val emoji: EmojiView) : ComposerSuggestion {
        override val key = "sticker/${emoji.id}"
    }

    data class Unicode(val emoji: UnicodeEmoji) : ComposerSuggestion {
        override val key = "unicode/${emoji.glyph}"
    }
}

internal fun ServerState.canSuggestEveryone(channelId: String = selected): Boolean =
    me.isNotEmpty() &&
        capabilities.everyoneMessage &&
        channels.any { it.id == channelId && it.kind in setOf("text", "voice") }

internal fun composerSuggestions(
    trigger: ComposerTrigger,
    server: ServerState,
    unicode: List<UnicodeEmoji>,
    channelId: String = server.selected,
): List<ComposerSuggestion> = buildList {
    val query = trigger.query
    if (trigger.people) {
        // This is a plain protocol token, not a synthetic user binding. Never offer it in DMs.
        if (server.canSuggestEveryone(channelId) && "everyone".startsWith(query, true))
            add(ComposerSuggestion.Everyone)
        server.members
            .filter { it.name.contains(query, true) || it.username.contains(query, true) }
            .sortedWith(
                compareBy<Person> { !it.name.startsWith(query, true) }
                    .thenBy { it.name.lowercase() }
            )
            .forEach { add(ComposerSuggestion.User(it)) }
    } else {
        // Server stickers retain priority, including when their alias matches a Unicode keyword.
        server.emojis
            .filter { it.name.contains(query, true) }
            .sortedWith(
                compareBy<EmojiView> { !it.name.startsWith(query, true) }
                    .thenBy { it.name.lowercase() }
            )
            .forEach { add(ComposerSuggestion.Sticker(it)) }
        matchingUnicodeEmoji(unicode, query).forEach { add(ComposerSuggestion.Unicode(it)) }
    }
}

internal fun ComposerDraft.pickSuggestion(
    suggestion: ComposerSuggestion,
    trigger: ComposerTrigger,
): ComposerDraft =
    when (suggestion) {
        ComposerSuggestion.Everyone -> insert("@everyone", trigger)
        is ComposerSuggestion.User ->
            insert("@${suggestion.person.name}", trigger, suggestion.person)
        is ComposerSuggestion.Sticker -> insert(":${suggestion.emoji.name}:", trigger)
        is ComposerSuggestion.Unicode -> insert(suggestion.emoji.glyph, trigger)
    }

@Composable
internal fun ComposerSuggestions(
    draft: ComposerDraft,
    server: ServerState,
    model: PapoViewModel,
    channelId: String = server.selected,
    onPick: (ComposerDraft) -> Unit,
) {
    val trigger = draft.trigger() ?: return
    val context = LocalContext.current
    val standard = remember { UnicodeEmojiCatalog.load(context) }
    val suggestions =
        remember(
            trigger,
            server.members,
            server.emojis,
            server.capabilities,
            server.me,
            server.channels,
            channelId,
        ) {
            composerSuggestions(trigger, server, standard, channelId)
        }
    if (suggestions.isEmpty()) return
    GlassPill(Modifier.fillMaxWidth().padding(bottom = 8.dp), radius = 24) {
        // A new token/query starts at the first match rather than inheriting another list's scroll.
        key(server.url, server.me, trigger.start, trigger.people, trigger.query) {
            LazyColumn(
                Modifier.heightIn(max = 216.dp).testTag("composer-suggestions"),
                contentPadding = PaddingValues(6.dp),
            ) {
                items(suggestions, key = { it.key }) { suggestion ->
                    ComposerSuggestionRow(suggestion, server, model) {
                        onPick(draft.pickSuggestion(suggestion, trigger))
                    }
                }
            }
        }
    }
}

@Composable
private fun ComposerSuggestionRow(
    suggestion: ComposerSuggestion,
    server: ServerState,
    model: PapoViewModel,
    onClick: () -> Unit,
) {
    when (suggestion) {
        ComposerSuggestion.Everyone ->
            SuggestionRow(
                "@everyone",
                ui(R.string.composer_everyone_description),
                { Text("@", style = MaterialTheme.typography.headlineSmall) },
                ui(R.string.mention_everyone),
                onClick,
            )
        is ComposerSuggestion.User -> {
            val person = suggestion.person
            SuggestionRow(
                person.name,
                person.username,
                { UserAvatar(person.id, person.name) },
                ui(R.string.mention_named, person.name),
                onClick,
            )
        }
        is ComposerSuggestion.Sticker -> {
            val emoji = suggestion.emoji
            SuggestionRow(
                ":${emoji.name}:",
                ui(R.string.server_sticker),
                { EmojiGlyph(server, EmojiChoice(emoji.id, true, emoji.name), 36, model) },
                ui(R.string.insert_sticker, emoji.name),
                onClick,
            )
        }
        is ComposerSuggestion.Unicode -> {
            val emoji = suggestion.emoji
            SuggestionRow(
                emoji.keywords,
                ui(R.string.emoji),
                { EmojiGlyph(server, EmojiChoice(emoji.glyph), 36, model) },
                ui(R.string.composer_insert_unicode, emoji.glyph),
                onClick,
            )
        }
    }
}

@Composable
private fun SuggestionRow(
    title: String,
    subtitle: String,
    leading: @Composable () -> Unit,
    label: String,
    onClick: () -> Unit,
) {
    ListItem(
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        leadingContent = leading,
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(onClickLabel = label, onClick = onClick),
    )
}
