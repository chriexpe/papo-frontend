package io.github.chriexpe.papo.compose

import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class MessageLinksTest {
    @Test fun copiesExactUrlsAndHidesOnlyDirectMediaWhileKeepingCaptionAndOtherLinks() {
        assertEquals(listOf("https://cdn.example/image_(1).gif?sig=a%2Bb&x=2","http://example.test/page"),messageLinks("See (https://cdn.example/image_(1).gif?sig=a%2Bb&x=2), then http://example.test/page.").map {it.url})
        val client=NativeClient.demo()
        try {
            client.dispatch(Action.SelectChannel("media"))
            val messages=client.poll().messages
            val portrait=messages.single {it.id=="media-7"}
            assertEquals(listOf("https://cdn.example.test/portrait.png?sig=fixture"),portrait.nativeLinks().map {it.url})
            assertTrue(portrait.visibleContent().contains("As dimensões"))
            assertTrue(portrait.visibleContent().contains("https://example.test/more"))
            assertFalse(portrait.visibleContent().contains("cdn.example.test"))
            assertTrue(messages.single {it.id=="media-6"}.embeds.single().let {!it.nativeMedia && it.sourceType=="custom"})
            assertFalse(messages.single {it.id=="media-10"}.embeds.single().nativeMedia)
        } finally {client.close()}
    }

    @Test fun publicGifRequestsHaveARealOriginAndDownloadsPublishTheExactBytes() = runBlocking {
        val source=publicGifSource("https://media.example.test/a.gif?sig=secret")
        assertEquals("https://media.example.test",source.origin)
        assertEquals("public/https://media.example.test/a.gif?sig=secret",source.cacheKey)
        assertTrue(source.headers.isEmpty())
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val file=context.cacheDir.resolve("download-${System.nanoTime()}.gif")
        file.writeBytes(context.assets.open("demo/animation.gif").use {it.readBytes()})
        try {
            val uri=saveMediaDownload(context,file,"Papo download fixture.gif","image/gif")
            try {
                assertArrayEquals(file.readBytes(),context.contentResolver.openInputStream(uri)!!.use {it.readBytes()})
                context.contentResolver.query(uri,arrayOf(android.provider.MediaStore.MediaColumns.IS_PENDING,android.provider.MediaStore.MediaColumns.MIME_TYPE),null,null,null)!!.use {cursor ->
                    assertTrue(cursor.moveToFirst());assertEquals(0,cursor.getInt(0));assertEquals("image/gif",cursor.getString(1))
                }
            } finally {context.contentResolver.delete(uri,null,null)}
        } finally {file.delete()}
    }
}
