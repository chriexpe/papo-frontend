package io.github.chriexpe.papo.compose

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.NativeClient
import org.junit.Assert.*
import org.junit.Test

class SearchEditorTest {
    @Test
    fun unicodeCaretCompletionReplacesTheWholeTokenAndKeepsExactlyOneSpace() {
        val client = NativeClient.demo()
        try {
            val value = TextFieldValue("🙂 de:an   after", TextRange(8))
            val hint =
                client.analyzeSearch(value.text, 8u).suggestions.first {
                    it.replacement == "de:@ana"
                }
            assertEquals(3u, hint.start)
            assertEquals(8u, hint.end)
            val next = applySearchSuggestion(value, hint)
            assertEquals("🙂 de:@ana after", next.text)
            assertEquals(TextRange(11), next.selection)
            val quoted = TextFieldValue("em:\"#geral\" tem:link", TextRange(8))
            val channel =
                client.analyzeSearch(quoted.text, 8u).suggestions.first {
                    it.replacement == "em:\"#geral\""
                }
            assertEquals("em:\"#geral\" tem:link", applySearchSuggestion(quoted, channel).text)
        } finally {
            client.shutdown()
            client.close()
        }
    }

    @Test
    fun searchUsesSharedFiltersAcrossConversationsAndClearsItsGeneration() {
        val client = NativeClient.demo()
        try {
            client.dispatch(Action.SelectChannel("dm-ana"))
            client.dispatch(Action.Search("de:@ana em:#geral", false))
            val result = client.poll()
            assertTrue(result.searchResults.isNotEmpty())
            assertTrue(
                result.searchResults.all { it.authorId == "ana" && it.channelId == "general" }
            )
            assertTrue(result.capabilities.canAdmin)
            assertTrue(result.capabilities.copy(banMembers = false).canBanMembers)
            assertFalse(result.capabilities.copy(manageServer = false).canBanMembers)
            assertFalse(
                result.capabilities
                    .copy(
                        manageServer = false,
                        manageChannels = false,
                        manageRoles = false,
                        banMembers = false,
                        manageWebhooks = false,
                    )
                    .canAdmin
            )
            assertTrue(client.serverIcon().isNotEmpty())
            client.dispatch(Action.ClearSearch)
            assertTrue(client.poll().searchResults.isEmpty())
        } finally {
            client.shutdown()
            client.close()
        }
    }
}
