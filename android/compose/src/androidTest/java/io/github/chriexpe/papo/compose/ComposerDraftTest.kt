package io.github.chriexpe.papo.compose

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import io.github.chriexpe.papo.core.NativeClient
import org.junit.Assert.*
import org.junit.Test

class ComposerDraftTest {
    @Test fun heldAuthorMentionAddsBoundaryAndPreservesAccountIdentity() {
        val client=NativeClient.demo()
        try {
            val ana=client.poll().members.first {it.id=="ana"}
            val draft=ComposerDraft(TextFieldValue("hello",TextRange(5))).mention(ana).mention(ana)
            assertEquals("hello @Ana @Ana ",draft.text)
            assertEquals(listOf("ana","ana"),draft.mentions.map {it.userId})
            assertTrue(draft.mentions.all {it.valid(draft.text)})
            assertEquals("hello @mention(<@ana>) @mention(<@ana>) ",client.encodeMessage(draft.text,draft.mentions.map {it.native(draft.text)}))
        } finally {client.shutdown();client.close()}
    }
    @Test fun suggestionsReplaceTheTokenAtCursorAndLeaveOneSpace() {
        val client=NativeClient.demo()
        try {
            val server=client.poll(); val ana=server.members.first {it.id=="ana"}
            val initial=ComposerDraft(TextFieldValue("hi @An   there",TextRange(6)))
            val picked=initial.insert("@Ana",initial.trigger(),ana)
            assertEquals("hi @Ana there",picked.text)
            assertEquals(TextRange(8),picked.value.selection)
            assertEquals("ana",picked.mentions.single().userId)
            assertNull(ComposerDraft(TextFieldValue("mail@An",TextRange(7))).trigger())
            assertNull(ComposerDraft(TextFieldValue("https:",TextRange(6))).trigger())
            assertNull(picked.trigger())
            val restored=ComposerDraft.restore(picked.text,picked.json())
            assertEquals(picked.mentions,restored.mentions)
        } finally {client.shutdown();client.close()}
    }
    @Test fun mentionIdentitySurvivesEditsAndEmojiUtf16Offsets() {
        val client=NativeClient.demo()
        try {
            val ana=client.poll().members.first {it.id=="ana"}
            val initial=ComposerDraft(TextFieldValue("🙂 @An",TextRange(6)))
            val picked=initial.insert("@Ana",initial.trigger(),ana)
            assertEquals(3,picked.mentions.single().start)
            assertEquals("🙂 @mention(<@ana>) ",client.encodeMessage(picked.text,picked.mentions.map {it.native(picked.text)}))
            client.dispatch(io.github.chriexpe.papo.core.Action.OpenDm("ana"))
            assertEquals("🙂 @mention(<@ana>) ",client.encodeMessage(picked.text,picked.mentions.map {it.native(picked.text)}))
            val shifted=picked.edited(TextFieldValue("hello "+picked.text,TextRange(0)))
            assertEquals(9,shifted.mentions.single().start)
            val altered=picked.edited(TextFieldValue("🙂 @Anna "))
            assertTrue(altered.mentions.isEmpty())
        } finally {client.shutdown();client.close()}
    }
    @Test fun inlineStickerMappingAndBackspaceKeepEditingAtomic() {
        val client=NativeClient.demo()
        try {
            val server=client.poll(); val ana=server.members.first {it.id=="ana"}
            val text=":papo_wave: @Ana "
            val draft=ComposerDraft(TextFieldValue(text,TextRange(11)),listOf(ComposerMention(12,"Ana",ana.id)))
            val stickers=composerStickers(text,server.emojis)
            val projection=ComposerProjection(draft,stickers,setOf("demo-wave"),Color.Blue)
            assertEquals("\u3000 @Ana ",projection.annotated.text)
            assertEquals(1,projection.positions.size)
            val offsets=(0..text.length).map {projection.mapping.originalToTransformed(it)}
            assertEquals(offsets.sorted(),offsets)
            assertEquals(11,projection.mapping.transformedToOriginal(1))
            assertTrue(projection.annotated.spanStyles.any {it.item.fontWeight==FontWeight.Bold && it.start==2})
            val deleted=draft.deleteSticker(TextFieldValue(text.removeRange(10,11),TextRange(10)),stickers.map {it.range})
            assertEquals(" @Ana ",deleted.text)
            assertEquals(TextRange(0),deleted.value.selection)
            assertEquals(1,deleted.mentions.single().start)
        } finally {client.shutdown();client.close()}
    }
}
