package io.github.chriexpe.papo.compose

import android.net.Uri
import io.github.chriexpe.papo.core.MediaRequest
import io.github.chriexpe.papo.core.MarkdownDocument
import io.github.chriexpe.papo.core.parseMessageMarkdown

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

internal fun ChatMessage.document():MarkdownDocument = if(markdown.source==content) markdown else parseMessageMarkdown(content)

internal fun ChatMessage.nativeLinks(): List<MessageLink> {
    // Code and named Markdown links are never implicit inline players. Keep
    // original source offsets for previews and exact signed URL spelling.
    val bare=document().nodes.flatMap {it.spans}.filter {it.kind=="text" && it.link.isBlank()}.flatMap {messageLinks(it.text)}.map {it.url}.toSet()
    return messageLinks(content).filter { link -> link.url in bare && run {
    val embed=embeds.firstOrNull {it.url==link.url}
    if(embed!=null) embed.nativeMedia else directMediaKind(link.url)!=null
    }}
}

internal fun ChatMessage.contextLinks():List<String> = document().nodes.flatMap {node ->
    messageLinks(node.text).map {it.url} + node.spans.flatMap {span ->
        if(span.link.isNotBlank()) listOf(span.link) else messageLinks(span.text).map {it.url}
    }
}.filter(::isPublicLink).distinct()

internal fun ChatMessage.visibleEmbeds():List<io.github.chriexpe.papo.core.EmbedView> {
    val doc=document()
    val images=doc.nodes.flatMap {it.spans}.filter {it.kind=="image"}.map {it.link}.toSet()
    val code=doc.nodes.flatMap {node -> messageLinks(node.text).map {it.url}+node.spans.filter {it.kind=="code"}.flatMap {messageLinks(it.text)}.map {it.url}}.toSet()
    val linked=doc.nodes.flatMap {it.spans}.filter {it.kind=="text"}.flatMap {span -> listOf(span.link)+messageLinks(span.text).map {it.url}}.toSet()
    return embeds.filter {it.sourceType=="custom" || (it.url !in images && (it.url !in code || it.url in linked))}
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
