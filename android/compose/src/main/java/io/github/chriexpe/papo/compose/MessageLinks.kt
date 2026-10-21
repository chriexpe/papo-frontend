package io.github.chriexpe.papo.compose

import android.net.Uri
import io.github.chriexpe.papo.core.MediaRequest

internal data class MessageLink(val url: String, val start: Int, val end: Int)

/** Keep the original spelling for copying, including signed query strings. */
internal fun messageLinks(text: String): List<MessageLink> = Regex("https?://[^\\s<>\"']+",RegexOption.IGNORE_CASE).findAll(text).mapNotNull { match ->
    var url=match.value.trimEnd('.',',',';', '!', '?')
    for((open,close) in listOf('(' to ')','[' to ']','{' to '}')) {
        while(url.endsWith(close) && url.count {it==close}>url.count {it==open}) url=url.dropLast(1)
    }
    url.takeIf(::isPublicLink)?.let {MessageLink(it,match.range.first,match.range.first+it.length)}
}.toList()

internal fun directMediaKind(url: String): String? {
    val uri=Uri.parse(url)
    val extension=uri.path.orEmpty().substringAfterLast('.',"").lowercase()
    val format=uri.getQueryParameter("format")?.lowercase()
    return when(extension) {
        "gif" -> "gif"
        "png","jpg","jpeg","webp","avif","bmp","heic","heif","tif","tiff","svg" -> "image"
        "mp4","m4v","webm","mov","mkv","avi","ogv","m3u8","3gp" -> "video"
        "mp3","m4a","aac","ogg","opus","flac","wav","weba" -> "audio"
        else -> when(format) {"gif" -> "gif"; "png","jpg","jpeg","webp","avif" -> "image"; else -> null}
    }
}

internal fun ChatMessage.nativeLinks(): List<MessageLink> = messageLinks(content).filter { link ->
    val embed=embeds.firstOrNull {it.url==link.url}
    if(embed!=null) embed.nativeMedia else directMediaKind(link.url)!=null
}

internal fun ChatMessage.visibleContent(): String {
    val hidden=nativeLinks()
    if(hidden.isEmpty()) return content
    return buildString {
        var end=0
        hidden.forEach {link -> append(content.substring(end,link.start));end=link.end}
        append(content.substring(end))
    }.trim()
}

internal fun ChatMessage.previewText():String = visibleContent().ifBlank {
    when {
        nativeLinks().isNotEmpty() || embeds.any {it.nativeMedia} -> "Media"
        attachments.isNotEmpty() -> attachments.first().name
        else -> "Message"
    }
}

internal fun publicMediaSource(url: String): MediaRequest {
    require(isPublicLink(url)) {"Invalid media address"}
    val uri=Uri.parse(url)
    val origin=uri.buildUpon().path("").query(null).fragment(null).build().toString().trimEnd('/')
    return MediaRequest(url,emptyMap(),"public/$url",origin)
}
