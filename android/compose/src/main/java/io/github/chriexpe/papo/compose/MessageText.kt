package io.github.chriexpe.papo.compose

import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.runtime.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

@Composable
internal fun MessageText(content: String, server: ServerState, model: PapoViewModel) {
    var detail by remember(server.url,server.me) {mutableStateOf<String?>(null)}
    val color=MaterialTheme.colorScheme.primary
    val customs=remember(server.emojis) { server.emojis.associateBy { it.name } }
    val pattern=remember {Regex("https?://[^\\s<>]+|@mention\\(<@(?<person>[^>\\s]+)>\\)|:(?<sticker>[A-Za-z0-9_+\\-]+):")}
    val matches=remember(content,customs) {pattern.findAll(content).filter {it.value.startsWith("http") || it.value.startsWith("@mention(") || customs.containsKey(it.groups["sticker"]?.value)}.toList()}
    val sticker=matches.isNotEmpty() && matches.all {it.value.startsWith(":")} && matches.size<=3 && pattern.replace(content,"").isBlank()
    val size=if(sticker) 72.sp else 24.sp
    val inline=matches.filter {it.value.startsWith(":")}.associate {m ->
        val e=customs.getValue(requireNotNull(m.groups["sticker"]?.value))
        e.id to InlineTextContent(Placeholder(size,size,PlaceholderVerticalAlign.TextCenter)) {
            Box(Modifier.fillMaxSize().testTag("message-sticker-${e.id}").clickable(onClickLabel="View sticker :${e.name}:") {detail=e.id},contentAlignment=Alignment.Center) {EmojiGlyph(server,EmojiChoice(e.id,true,e.name),if(sticker) 72 else 24,model)}
        }
    }
    val text=remember(content,matches,color,server.members) { buildAnnotatedString {
        var end=0
        matches.forEach {m ->
            append(content.substring(end,m.range.first))
            if(m.value.startsWith("http")) withLink(LinkAnnotation.Url(m.value,TextLinkStyles(SpanStyle(color=color)))) {append(m.value)}
            else if(m.value.startsWith("@mention(")) {
                val id=m.groups["person"]?.value.orEmpty()
                withStyle(SpanStyle(color=color,fontWeight=FontWeight.Bold)) {append("@${server.members.firstOrNull {it.id==id}?.name ?: id}")}
            }
            else appendInlineContent(customs.getValue(requireNotNull(m.groups["sticker"]?.value)).id,m.value)
            end=m.range.last+1
        }
        append(content.substring(end))
    } }
    SelectionContainer {Text(text,inlineContent=inline,style=MaterialTheme.typography.bodyLarge,lineHeight=if(sticker) 80.sp else 26.sp)}
    detail?.let {id -> StickerDetails(server,model,id,onDismiss={detail=null})}
}
