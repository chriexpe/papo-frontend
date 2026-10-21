package io.github.chriexpe.papo.compose

import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.NativeClient
import org.junit.Assert.*
import org.junit.Test

class ExpressionLibraryTest {
    @Test fun favoritesRecentAndFrequencySurviveReloadAndStayAccountScoped() {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val owner="test/${System.nanoTime()}"
        val library=ExpressionLibrary(context)
        val a=GifChoice("giphy:first","First",providerId="first")
        val b=GifChoice("giphy:second","Second",providerId="second")
        try {
            library.favorite(owner,a); library.recent(owner,a); library.recent(owner,b); library.recent(owner,a)
            library.used(owner,listOf(EmojiChoice("👍"),EmojiChoice("wave",true,"wave"),EmojiChoice("wave",true,"wave")))
            val restored=ExpressionLibrary(context)
            assertEquals(listOf(a),restored.state(owner).favorites)
            assertEquals(listOf(a,b),restored.state(owner).recent)
            assertEquals("c:wave",restored.state(owner).usage.first().first.key)
            assertTrue(restored.state("other/$owner").favorites.isEmpty())
            restored.favorite(owner,a)
            assertTrue(restored.state(owner).favorites.isEmpty())
            assertEquals(listOf(a,b),restored.state(owner).recent)
        } finally {context.getSharedPreferences("compose-expressions",0).edit().remove(owner).commit()}
    }
    @Test fun generatedBridgeCarriesAnimatedServerEmojiAndCustomReactionPeople() {
        val client=NativeClient.demo()
        try {
            val server=client.poll()
            val emoji=server.emojis.single()
            assertEquals("papo_wave",emoji.name)
            assertTrue(client.emojiImage(emoji.id).size>30)
            val m=server.messages.last()
            client.dispatch(Action.React(server.selected,m.id,emoji.id,true,true))
            val reaction=client.poll().messages.last().reactions.first {it.custom}
            assertEquals(emoji.id,reaction.emoji); assertTrue(reaction.mine)
            client.dispatch(Action.ReactionDetails(server.selected,m.id,false))
            val people=client.poll().reactionDetails.single {it.messageId==m.id}
            assertTrue(people.groups.first {it.custom}.users.any {it.id==server.me && it.name=="Christian"})
            assertEquals(1,client.gifPage("",0u).items.size)
            assertEquals("demo-motion",client.gifItem("demo-motion").id)
        } finally {client.shutdown();client.close()}
    }
    @Test fun usageRecognizesServerStickersAndPhoneEmojiAsWholeGraphemes() {
        val client=NativeClient.demo()
        try {
            val choices=messageEmoji(":papo_wave: 👩🏽‍💻 ❤️ 🇧🇷",client.poll())
            assertEquals(4,choices.size)
            assertEquals("c:demo-wave",choices[0].key)
            assertTrue(choices.any {it.value=="👩🏽‍💻"})
            assertTrue(choices.any {it.value=="❤️"})
        } finally {client.shutdown();client.close()}
    }
}
