package io.github.chriexpe.papo.compose

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.NativeClient
import org.junit.Assert.*
import org.junit.Test

class ComposerSuggestionsTest {
    @Test
    fun everyoneFollowsCurrentPermissionAndConversationWithoutBindingToAUser() {
        val client = NativeClient.demo()
        try {
            val server = client.poll().copy(selectedChannel = "general")
            val draft = ComposerDraft(TextFieldValue("@Ev", TextRange(3)))
            val trigger = draft.trigger()!!
            fun choices(state: ServerState, channel: String = state.selected) =
                composerSuggestions(trigger, state, emptyList(), channel)
            assertEquals(ComposerSuggestion.Everyone, choices(server).first())
            assertFalse(
                choices(
                        server.copy(
                            capabilities = server.capabilities.copy(everyoneMessage = false)
                        )
                    )
                    .contains(ComposerSuggestion.Everyone)
            )
            listOf("dm-ana", "projects", "missing", "").forEach {
                assertFalse(choices(server, it).contains(ComposerSuggestion.Everyone))
            }
            assertFalse(choices(server.copy(me = "")).contains(ComposerSuggestion.Everyone))
            val person =
                server.members
                    .first()
                    .copy(id = "real-user", name = "everyone", username = "everyone")
            val colliding = choices(server.copy(members = listOf(person)))
            assertEquals(
                listOf(ComposerSuggestion.Everyone, ComposerSuggestion.User(person)),
                colliding,
            )

            val picked = draft.pickSuggestion(ComposerSuggestion.Everyone, trigger)
            assertEquals("@everyone ", picked.text)
            assertEquals(TextRange(10), picked.value.selection)
            assertTrue(picked.mentions.isEmpty())
            assertEquals("@everyone ", client.encodeMessage(picked.text, emptyList()))
            val boundUser = draft.pickSuggestion(ComposerSuggestion.User(person), trigger)
            assertEquals(
                "@mention(<@real-user>) ",
                client.encodeMessage(
                    boundUser.text,
                    boundUser.mentions.map { it.native(boundUser.text) },
                ),
            )
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun sharedCatalogMatchesAliasesAccentsAndGlyphsWithServerStickersFirst() {
        val client = NativeClient.demo()
        try {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val catalog = UnicodeEmojiCatalog.load(context)
            assertSame(catalog, UnicodeEmojiCatalog.load(context))
            val server = client.poll()
            val thumb = catalog.first { it.glyph == "👍" }
            assertEquals(listOf(thumb), matchingUnicodeEmoji(catalog, "THUMBS_UP"))
            assertEquals(listOf(thumb), matchingUnicodeEmoji(catalog, "thumbs-up"))
            assertEquals(listOf(thumb), matchingUnicodeEmoji(catalog, "👍"))
            assertEquals(
                1,
                matchingUnicodeEmoji(listOf(UnicodeEmoji("✈️", "avião airplane")), "aviao").size,
            )
            val duplicates =
                UnicodeEmojiCatalog.decode(
                    """[{"glyph":"👍","keywords":"thumbs up"},{"glyph":"👍","keywords":"like"}]"""
                )
            assertEquals(1, duplicates.size)
            assertEquals(duplicates, matchingUnicodeEmoji(duplicates, "like"))
            val sticker = server.emojis.first().copy(name = "thumbs_up")
            val choices =
                composerSuggestions(
                    ComposerTrigger(0, 10, "thumbs_up", false),
                    server.copy(emojis = listOf(sticker)),
                    catalog + thumb,
                )
            assertEquals(ComposerSuggestion.Sticker(sticker), choices.first())
            assertEquals(
                1,
                choices.filterIsInstance<ComposerSuggestion.Unicode>().count {
                    it.emoji.glyph == "👍"
                },
            )
            assertTrue(
                composerSuggestions(ComposerTrigger(0, 12, "zzzznomatch", false), server, catalog)
                    .isEmpty()
            )
            assertTrue(
                composerSuggestions(ComposerTrigger(0, 1, "", false), server, catalog).first()
                    is ComposerSuggestion.Sticker
            )
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun unicodeReplacementInsideShortcodeKeepsAdjacentMentionIdentityAndCaret() {
        val client = NativeClient.demo()
        try {
            val text = "🙂 :thumbs_up: \t@Ana"
            val draft =
                ComposerDraft(
                    TextFieldValue(
                        text,
                        TextRange(text.indexOf('_')),
                        TextRange(3, text.indexOf('_')),
                    ),
                    listOf(ComposerMention(text.indexOf("@Ana"), "Ana", "ana")),
                )
            val picked =
                draft.pickSuggestion(
                    ComposerSuggestion.Unicode(UnicodeEmoji("👍", "thumbs up")),
                    draft.trigger()!!,
                )
            assertEquals("🙂 👍 @Ana", picked.text)
            assertEquals(TextRange(6), picked.value.selection)
            assertNull(picked.value.composition)
            assertEquals("ana", picked.mentions.single().userId)
            assertEquals(
                "🙂 👍 @mention(<@ana>)",
                client.encodeMessage(picked.text, picked.mentions.map { it.native(picked.text) }),
            )
            assertEquals(
                picked.mentions,
                ComposerDraft.restore(picked.text, picked.json()).mentions,
            )
            // Merely moving the IME's composition/selection must not rewrite text or bindings.
            val composing = picked.edited(picked.value.copy(composition = TextRange(0, 2)))
            assertEquals(picked.text, composing.text)
            assertEquals(picked.mentions, composing.mentions)
            assertNull(draft.copy(value = draft.value.copy(selection = TextRange(3, 5))).trigger())
            assertNull(ComposerDraft(TextFieldValue(":thumbs up", TextRange(10))).trigger())
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun everyoneHighlightUsesProjectedOffsetsAndDoesNotCreateMentionBindings() {
        val client = NativeClient.demo()
        try {
            val server = client.poll()
            val draft = ComposerDraft(TextFieldValue(":papo_wave: @everyone"))
            val stickers = composerStickers(draft.text, server.emojis)
            val highlighted =
                ComposerProjection(draft, stickers, setOf("demo-wave"), Color.Blue, true)
            assertEquals("\u3000 @everyone", highlighted.annotated.text)
            assertTrue(
                highlighted.annotated.spanStyles.any {
                    it.start == 2 && it.end == 11 && it.item.fontWeight == FontWeight.Bold
                }
            )
            assertFalse(
                ComposerProjection(draft, stickers, setOf("demo-wave"), Color.Blue, false)
                    .annotated
                    .spanStyles
                    .any { it.item.fontWeight == FontWeight.Bold }
            )
            assertTrue(draft.mentions.isEmpty())
        } finally {
            client.shutdown()
            client.close()
        }
    }
}
