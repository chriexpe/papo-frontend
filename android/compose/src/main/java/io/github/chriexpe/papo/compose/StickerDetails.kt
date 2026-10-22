@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Star
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun StickerDetails(server:ServerState,model:PapoViewModel,id:String,onDismiss:()->Unit) {
    val sticker=server.emojis.firstOrNull {it.id==id}
    val states by model.expressions.states.collectAsState()
    val owner=model.expressions.owner(server)
    val library=states[owner] ?: model.expressions.state(owner)
    val favorite=id in library.favoriteStickers
    val clipboard=LocalClipboardManager.current
    val scope=rememberCoroutineScope()
    var copied by remember(id) {mutableStateOf(false)}
    var error by remember(id) {mutableStateOf("")}
    var saving by remember(id) {mutableStateOf(false)}
    ModalBottomSheet(onDismissRequest=onDismiss) {
        Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal=24.dp).padding(bottom=24.dp).testTag("sticker-details"),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(16.dp)) {
            if(sticker==null) Text(ui(R.string.this_sticker_is_no_longer_available),style=MaterialTheme.typography.bodyLarge)
            else {
                Box(Modifier.size(112.dp),contentAlignment=Alignment.Center) {EmojiGlyph(server,EmojiChoice(id,true,sticker.name),96,model)}
                Text(":${sticker.name}:",style=MaterialTheme.typography.titleLarge)
                Text(ui(R.string.favorite_stickers_stay_in_your_quick_reactions),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                    FilledTonalButton(onClick={scope.launch {
                        saving=true; error=""
                        try {withContext(Dispatchers.IO) {model.expressions.favoriteSticker(owner,id)}}
                        catch(e:CancellationException) {throw e}
                        catch(_:Exception) {error=ui(R.string.couldn_t_update_this_favorite_try_again)}
                        finally {saving=false}
                    }},enabled=!saving,modifier=Modifier.weight(1f)) {
                        Icon(if(favorite) Icons.Outlined.Star else Icons.Outlined.StarBorder,null,Modifier.size(20.dp)); Spacer(Modifier.width(8.dp)); Text(if(favorite) "Favorited" else "Favorite")
                    }
                    OutlinedButton(onClick={clipboard.setText(AnnotatedString(sticker.name)); copied=true},modifier=Modifier.weight(1f)) {
                        Icon(Icons.Outlined.ContentCopy,null,Modifier.size(20.dp)); Spacer(Modifier.width(8.dp)); Text(if(copied) "Copied" else ui(R.string.copy_name))
                    }
                }
                if(error.isNotBlank()) Text(error,color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall)
            }
        }
    }
}
