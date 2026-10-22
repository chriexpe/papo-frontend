package io.github.chriexpe.papo.compose

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import io.github.chriexpe.papo.core.NativeClient
import io.github.chriexpe.papo.core.parseMessageMarkdown
import org.junit.Assert.*
import org.junit.Test

class MarkdownTest {
    @Test fun sharedParserPublishesStructuredBlocksAndSafeLinks() {
        val doc=parseMessageMarkdown("# Heading\n\n**bold** *italic* ~~gone~~ `:literal:`\n\n> quote\n\n3. first\n4. second\n\n- [x] done\n- [ ] todo\n\n```kotlin\nval url = \"https://cdn.test/code.mp4\"\n```\n\n| A | B |\n| :-- | --: |\n| one | two |\n\n![alt](https://cdn.test/pic.png) [unsafe](javascript:alert(1))\n\n---")
        assertTrue(doc.nodes.map {it.kind}.containsAll(listOf("heading","paragraph","quote","list","code","table","rule")))
        assertEquals("kotlin",doc.nodes.single {it.kind=="code"}.language)
        val spans=doc.nodes.flatMap {it.spans}
        assertTrue(spans.any {it.text=="bold" && it.bold})
        assertTrue(spans.any {it.text=="italic" && it.italic})
        assertTrue(spans.any {it.text=="gone" && it.strike})
        assertTrue(spans.any {it.kind=="code" && it.text==":literal:"})
        assertTrue(spans.any {it.kind=="image" && it.link=="https://cdn.test/pic.png"})
        assertFalse(spans.any {it.link.startsWith("javascript:")})
        assertEquals(listOf("left","right"),doc.nodes.single {it.kind=="table"}.alignments)
        assertEquals(listOf(true,false),doc.nodes.filter {it.task!=null}.map {it.task})
        assertEquals("https://chat.test/path",markdownHref("https://chat.test","/path"))
        assertEquals("mailto:a@example.test",markdownHref("https://chat.test","mailto:a@example.test"))
        listOf("javascript:alert(1)","data:image/png,x","file:///etc/passwd").forEach {assertNull(markdownHref("https://chat.test",it))}
    }

    @Test fun codeAndNamedLinksDoNotTurnIntoHiddenNativeMedia() {
        val client=NativeClient.demo()
        try {
            val content="Caption https://cdn.test/native.gif?sig=a%2Bb&x=2\n\n`https://cdn.test/literal.mp4` [label](https://cdn.test/named.png)\n\n```\nhttps://cdn.test/fenced.mp4\n```"
            val message=client.poll().messages.first().copy(content=content,markdown=parseMessageMarkdown(content),embeds=emptyList())
            assertEquals(listOf("https://cdn.test/native.gif?sig=a%2Bb&x=2"),message.nativeLinks().map {it.url})
            assertTrue(message.visibleContent().contains("literal.mp4"))
            assertTrue(message.visibleContent().contains("[label]"))
            assertEquals(setOf("https://cdn.test/native.gif?sig=a%2Bb&x=2","https://cdn.test/literal.mp4","https://cdn.test/named.png","https://cdn.test/fenced.mp4"),message.contextLinks().toSet())
        } finally {client.close()}
    }

    @Test fun formattingPairsMarkersAndPreservesUnicodeAndMentionIdentity() {
        val client=NativeClient.demo()
        val formatter=ComposerFormatting()
        try {
            val person=client.poll().members.first {it.id=="ana"}
            var draft=ComposerDraft().mention(person)
            fun type(text:String) {text.codePoints().toArray().forEach {c ->val token=String(Character.toChars(c));val at=draft.value.selection.end; val next=draft.text.replaceRange(at,at,token);draft=formatter.edit(draft,TextFieldValue(next,TextRange(at+token.length)),emptyList())}}
            type("🙂 **hello")
            assertEquals("@Ana 🙂 **hello**",draft.text)
            assertEquals("ana",draft.mentions.single().userId)
            draft=requireNotNull(formatter.skip(draft))
            assertEquals("@Ana 🙂 **hello** ",draft.text)
            assertEquals(draft.text.length,draft.value.selection.end)
            type("~~gone~~")
            assertEquals("@Ana 🙂 **hello** ~~gone~~ ",draft.text)
            assertEquals("@mention(<@ana>) 🙂 **hello** ~~gone~~ ",client.encodeMessage(draft.text,draft.mentions.map {it.native(draft.text)}))
        } finally {formatter.close();client.close()}
    }

    @Test fun imeCompositionMovesAnExistingCloserWithoutRewritingTheComposingWord() {
        val formatter=ComposerFormatting()
        try {
            var draft=ComposerDraft(TextFieldValue("*",TextRange(1)))
            draft=formatter.edit(draft,TextFieldValue("**",TextRange(2)),emptyList())
            val composing=TextFieldValue("**cafe**",TextRange(6),TextRange(2,6))
            draft=formatter.edit(draft,composing,emptyList())
            assertEquals(composing,draft.value)
            val corrected=TextFieldValue("**café**",TextRange(6),TextRange(2,6))
            draft=formatter.edit(draft,corrected,emptyList())
            assertEquals(corrected,draft.value)
            draft=formatter.edit(draft,corrected.copy(composition=null),emptyList())
            draft=formatter.edit(draft,TextFieldValue("**café***",TextRange(7)),emptyList())
            draft=formatter.edit(draft,TextFieldValue("**café***",TextRange(8)),emptyList())
            assertEquals("**café** ",draft.text)
        } finally {formatter.close()}
    }

    @Test fun compositionSelectionPasteAndProgrammaticChangesDoNotRewriteText() {
        val formatter=ComposerFormatting()
        try {
            val empty=ComposerDraft()
            val incomplete=TextFieldValue("\uD83D",TextRange(1))
            assertEquals(incomplete,formatter.edit(empty,incomplete,emptyList()).value)
            val composing=TextFieldValue("**",TextRange(2),TextRange(0,2))
            assertEquals(composing,formatter.edit(empty,composing,emptyList()).value)
            val committed=ComposerDraft(composing)
            assertEquals("**",formatter.edit(committed,TextFieldValue("**",TextRange(2)),emptyList()).text)
            assertEquals("**pasted",formatter.edit(empty,TextFieldValue("**pasted",TextRange(8)),emptyList()).text)
            val selected=ComposerDraft(TextFieldValue("replace",TextRange(0,7)))
            assertEquals("`",formatter.edit(selected,TextFieldValue("`",TextRange(1)),emptyList()).text)
            var draft=ComposerDraft(TextFieldValue("*",TextRange(1)))
            draft=formatter.edit(draft,TextFieldValue("**",TextRange(2)),emptyList())
            assertEquals("****",draft.text)
            val reset=ComposerDraft(TextFieldValue("unrelated **tail**",TextRange(10)))
            assertNull(formatter.skip(reset))
        } finally {formatter.close()}
    }
}
