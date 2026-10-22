@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import coil3.request.ImageRequest
import io.github.chriexpe.papo.core.Action
import io.github.chriexpe.papo.core.MediaRequest
import io.github.chriexpe.papo.core.MediaResource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.last
import java.time.OffsetDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

internal data class ProfileContext(val server:ServerState,val model:PapoViewModel)
internal val LocalProfileContext=compositionLocalOf<ProfileContext?> {null}
internal val LocalOpenProfile=compositionLocalOf<(String)->Unit> {{}}

/** Avatar bytes stay out of frequently polled snapshots. Both encoded and
 * decoded images are scoped to the server, account, user and image revision. */
@Composable
internal fun UserAvatar(id:String,name:String,modifier:Modifier=Modifier,shape:androidx.compose.ui.graphics.Shape=CircleShape) {
    val profileContext=LocalProfileContext.current
    val profile=profileContext?.server?.profiles?.firstOrNull {it.id==id}
    val revision=profile?.avatarRevision.orEmpty()
    val context=LocalContext.current
    val image=remember(profileContext?.server?.url,profileContext?.server?.me,id,revision) {
        if(profileContext==null || revision.isBlank()) null
        else CachedImage("${profileContext.server.url}/${profileContext.server.me}/avatar/$id/$revision") {profileContext.model.avatarImage(profileContext.server,id,revision)}
    }
    key(profileContext?.server?.url,profileContext?.server?.me,id,revision) {
        var loaded by remember {mutableStateOf(false)}
        Box(modifier.size(40.dp).clip(shape).testTag("avatar-$id").semantics {
            contentDescription=ui(R.string.member_avatar,name);stateDescription=if(loaded) ui(R.string.avatar_loaded) else uiLabel("Initials")
        },contentAlignment=Alignment.Center) {
            if(!loaded) Avatar(name,Modifier.fillMaxSize(),shape)
            if(image!=null) ManagedAsyncImage(
                ImageRequest.Builder(context).data(image).size(256).build(),null,MediaImages.loader(context),
                modifier=Modifier.fillMaxSize(),contentScale=ContentScale.Crop,onSuccess={loaded=true},onError={loaded=false},retentionKey="avatar/$id/$revision")
        }
    }
}

@Composable
internal fun ProfileBanner(server:ServerState,model:PapoViewModel,id:String,revision:String,modifier:Modifier) {
    val context=LocalContext.current
    val image=remember(server.url,server.me,id,revision) {
        if(revision.isBlank()) null else CachedImage("${server.url}/${server.me}/banner/$id/$revision") {
            MediaCache.get(context).images(model.mediaRequest(server.url,MediaResource.ProfileBanner(id))).last()
        }
    }
    Box(modifier.background(MaterialTheme.colorScheme.primary.copy(alpha=.16f))) {
        image?.let {ManagedAsyncImage(it,ui(R.string.profile_banner),MediaImages.loader(context),Modifier.fillMaxSize(),ContentScale.Crop)}
    }
}

internal fun roleColor(raw:String,fallback:Color):Color = runCatching {Color(android.graphics.Color.parseColor(raw))}.getOrDefault(fallback)

@Composable
internal fun userNameColor(id:String):Color = roleColor(LocalProfileContext.current?.server?.members?.firstOrNull {it.id==id}?.roleColor.orEmpty(),MaterialTheme.colorScheme.onSurface)

@Composable
internal fun UserProfileSheet(server:ServerState,model:PapoViewModel,id:String,onDismiss:()->Unit,onMessage:(String)->Unit) {
    val person=server.members.firstOrNull {it.id==id}
    val profile=server.profiles.firstOrNull {it.id==id}
    val name=person?.name ?: id
    val username=person?.username.orEmpty()
    val clipboard=LocalClipboardManager.current
    var resetLink by remember(id) {mutableStateOf(false)}
    var copied by remember(id) {mutableStateOf(false)}
    LaunchedEffect(server.url,server.me,id) {model.command(Action.LoadProfile(id),server.url)}
    ModalBottomSheet(onDismissRequest=onDismiss,dragHandle=null,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true)) {
        LazyColumn(Modifier.fillMaxWidth().heightIn(max=androidx.compose.ui.platform.LocalConfiguration.current.screenHeightDp.dp*.88f).testTag("user-profile"),contentPadding=PaddingValues(bottom=24.dp)) {
            item {
                BoxWithConstraints(Modifier.fillMaxWidth()) {
                    val bannerHeight=maxWidth/3
                    ProfileBanner(server,model,id,profile?.banner.orEmpty(),Modifier.fillMaxWidth().height(bannerHeight))
                    Box(Modifier.align(Alignment.TopCenter).padding(top=8.dp).size(32.dp,4.dp).background(MaterialTheme.colorScheme.onSurface.copy(alpha=.3f),CircleShape))
                    GlassPill(Modifier.align(Alignment.TopEnd).padding(8.dp)) {
                        IconButton(onClick=onDismiss) {Icon(Icons.Outlined.Close,ui(R.string.close_profile))}
                    }
                    Row(Modifier.padding(top=bannerHeight-36.dp,start=20.dp,end=20.dp),verticalAlignment=Alignment.Bottom,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                        Box(Modifier.size(88.dp)) {
                            UserAvatar(id,name,Modifier.size(88.dp).border(4.dp,MaterialTheme.colorScheme.surfaceContainerLow,CircleShape).padding(4.dp))
                            val presence=person?.presence ?: "offline"
                            val color=when(presence) {"online"->Color(0xFF50B77B);"away"->Color(0xFFF0B63F);"busy"->Color(0xFFDC6666);else->MaterialTheme.colorScheme.outline}
                            Box(Modifier.align(Alignment.BottomEnd).padding(4.dp).size(18.dp).background(color,CircleShape).border(3.dp,MaterialTheme.colorScheme.surfaceContainerLow,CircleShape).semantics {contentDescription=uiLabel(presence.replaceFirstChar(Char::uppercase))})
                        }
                        if(!profile?.statusMessage.isNullOrBlank()) Surface(shape=RoundedCornerShape(16.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh,modifier=Modifier.padding(bottom=6.dp).weight(1f,fill=false)) {
                            Text(profile!!.statusMessage,Modifier.padding(horizontal=12.dp,vertical=8.dp),style=MaterialTheme.typography.bodySmall,maxLines=3,overflow=TextOverflow.Ellipsis)
                        }
                    }
                }
                Column(Modifier.padding(horizontal=24.dp,vertical=12.dp),verticalArrangement=Arrangement.spacedBy(2.dp)) {
                    SelectionContainer {Text(name,color=userNameColor(id),style=MaterialTheme.typography.headlineSmall,fontWeight=FontWeight.SemiBold)}
                    if(username.isNotBlank()) Text("@$username",color=MaterialTheme.colorScheme.onSurfaceVariant,style=MaterialTheme.typography.bodyMedium)
                }
            }
            if(profile?.loaded==false && server.connection=="online") item {
                LinearProgressIndicator(Modifier.fillMaxWidth().padding(horizontal=24.dp).semantics {contentDescription=ui(R.string.loading_profile)})
            }
            if(!profile?.roles.isNullOrEmpty()) item {
                ProfileSection(ui(R.string.roles)) {
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        profile!!.roles.forEach {role -> Surface(shape=RoundedCornerShape(12.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh) {
                            Row(Modifier.padding(horizontal=10.dp,vertical=7.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                                Box(Modifier.size(8.dp).background(roleColor(role.color,MaterialTheme.colorScheme.primary),CircleShape))
                                Text(role.name,style=MaterialTheme.typography.labelMedium)
                            }
                        } }
                    }
                }
            }
            if(!profile?.description.isNullOrBlank() || !profile?.createdAt.isNullOrBlank()) item {
                ProfileSection(ui(R.string.about_2)) {
                    if(!profile?.description.isNullOrBlank()) SelectionContainer {Text(profile!!.description,style=MaterialTheme.typography.bodyMedium)}
                    val date=remember(profile?.createdAt,AppLanguage.locale) {runCatching {OffsetDateTime.parse(profile?.createdAt).format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(AppLanguage.locale))}.getOrNull()}
                    date?.let {Text(ui(R.string.member_since_1_s, it),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
                }
            }
            item {
                HorizontalDivider(color=MaterialTheme.colorScheme.outlineVariant.copy(alpha=.5f))
                Column(Modifier.padding(horizontal=12.dp).navigationBarsPadding()) {
                    if(id!=server.me) ListItem(headlineContent={Text(ui(R.string.message_person,if(username.isBlank()) name else "@$username"))},leadingContent={Icon(Icons.Outlined.ChatBubbleOutline,null)},trailingContent={Icon(Icons.Outlined.ChevronRight,null)},
                        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {onMessage(id)})
                    if(server.canResetPasswords && !server.isDemo && id!=server.me) ListItem(headlineContent={Text(ui(R.string.generate_recovery_link_2))},leadingContent={Icon(Icons.Outlined.Key,null)},
                        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {resetLink=true})
                    if(username.isNotBlank()) ListItem(headlineContent={Text(if(copied) ui(R.string.copied_username) else ui(R.string.copy_username))},leadingContent={Icon(Icons.Outlined.ContentCopy,null)},
                        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {clipboard.setText(AnnotatedString(username));copied=true})
                }
            }
        }
    }
    if(resetLink) GenerateRecoveryLink(server,model,id,name) {resetLink=false}
}

@Composable
private fun ProfileSection(label:String,content:@Composable ColumnScope.()->Unit) {
    HorizontalDivider(color=MaterialTheme.colorScheme.outlineVariant.copy(alpha=.5f))
    Column(Modifier.padding(horizontal=24.dp,vertical=16.dp),verticalArrangement=Arrangement.spacedBy(10.dp)) {
        Text(label,style=MaterialTheme.typography.labelSmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
        content()
    }
}

internal val LocalOpenExternalProfile=compositionLocalOf<(io.github.chriexpe.papo.core.ExternalAuthorView)->Unit> {{}}

/** External identities have no Papo user id. Only the documented webhook
 * avatar route may be resolved relative to the connected server. No auth
 * cookies are sent to the external image/profile URL. */
internal fun externalAvatarUrl(server:String,raw:String):String? {
    if(raw.startsWith("/webhooks/") && Regex("^/webhooks/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/avatar$").matches(raw)) {
        return runCatching {java.net.URI(server).resolve(raw).toString()}.getOrNull()?.let(::externalProfileUrl)
    }
    return externalProfileUrl(raw)
}
internal fun externalProfileUrl(raw:String):String? = runCatching {
    val uri=java.net.URI(raw)
    raw.takeIf {uri.scheme in listOf("http","https") && !uri.host.isNullOrBlank() && uri.userInfo==null}
}.getOrNull()

@Composable
internal fun ExternalAvatar(server:ServerState,author:io.github.chriexpe.papo.core.ExternalAuthorView,modifier:Modifier=Modifier) {
    val context=LocalContext.current
    val url=remember(server.url,author.avatarUrl) {
        if(server.isDemo && author.avatarUrl.startsWith("file:///android_asset/demo/")) author.avatarUrl
        else externalAvatarUrl(server.url,author.avatarUrl)
    }
    val bytes by produceState<ByteArray?>(null,server.url,server.me,author.source,author.id,url) {
        value=null
        if(url!=null) try {
            val source=if(server.isDemo) publicGifSource(url) else publicMediaSource(url).let {it.copy(cacheKey="${server.url}/${server.me}/external/${author.source}/${author.id}/$url")}
            MediaCache.get(context).images(source).collect {value=it}
        } catch(e:CancellationException) {throw e} catch(_:Exception) { }
    }
    val image=bytes
    var loaded by remember(url) {mutableStateOf(false)}
    Box(modifier.size(40.dp).clip(CircleShape).testTag("external-avatar-${author.source}-${author.id}").semantics {
        contentDescription=ui(R.string.member_avatar,author.name);stateDescription=if(loaded) ui(R.string.avatar_loaded) else uiLabel("Initials")
    }) {
        if(!loaded) Avatar(author.name,Modifier.fillMaxSize())
        image?.let {ManagedAsyncImage(it,null,MediaImages.loader(context),Modifier.fillMaxSize(),ContentScale.Crop,
            onSuccess={loaded=true},onError={loaded=false},retentionKey="external/${author.source}/${author.id}/$url/${bytes?.contentHashCode()}")}
    }
}

@Composable
internal fun AppBadge() {
    Surface(shape=RoundedCornerShape(5.dp),color=MaterialTheme.colorScheme.surfaceContainerHighest) {
        Text("APP",Modifier.padding(horizontal=5.dp,vertical=2.dp),style=MaterialTheme.typography.labelSmall,fontWeight=FontWeight.Bold)
    }
}

@Composable
internal fun ExternalProfileSheet(server:ServerState,author:io.github.chriexpe.papo.core.ExternalAuthorView,onDismiss:()->Unit) {
    val uriHandler=androidx.compose.ui.platform.LocalUriHandler.current
    val clipboard=LocalClipboardManager.current
    var copied by remember(author) {mutableStateOf(false)}
    val profile=externalProfileUrl(author.profileUrl)
    ModalBottomSheet(onDismissRequest=onDismiss,dragHandle=null,sheetState=rememberModalBottomSheetState(skipPartiallyExpanded=true)) {
        LazyColumn(Modifier.fillMaxWidth().heightIn(max=androidx.compose.ui.platform.LocalConfiguration.current.screenHeightDp.dp*.88f).testTag("external-profile"),contentPadding=PaddingValues(bottom=24.dp)) {
            item {
                Box(Modifier.fillMaxWidth().height(112.dp).background(MaterialTheme.colorScheme.primary.copy(alpha=.16f))) {
                    Box(Modifier.align(Alignment.TopCenter).padding(top=8.dp).size(32.dp,4.dp).background(MaterialTheme.colorScheme.onSurface.copy(alpha=.3f),CircleShape))
                    GlassPill(Modifier.align(Alignment.TopEnd).padding(8.dp)) {IconButton(onClick=onDismiss) {Icon(Icons.Outlined.Close,ui(R.string.close_profile))}}
                }
                ExternalAvatar(server,author,Modifier.padding(start=20.dp).offset(y=(-36).dp).size(88.dp).border(4.dp,MaterialTheme.colorScheme.surfaceContainerLow,CircleShape).padding(4.dp))
                Column(Modifier.offset(y=(-24).dp).padding(horizontal=24.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(10.dp)) {
                        SelectionContainer(Modifier.weight(1f,fill=false)) {Text(author.name,style=MaterialTheme.typography.headlineSmall,fontWeight=FontWeight.SemiBold)}
                        AppBadge()
                    }
                    Text(ui(R.string.via_provider,author.source),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                }
                HorizontalDivider(color=MaterialTheme.colorScheme.outlineVariant.copy(alpha=.5f))
                Text(ui(R.string.this_identity_is_provided_by_an_app_integration),Modifier.padding(horizontal=24.dp,vertical=20.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
            }
            item {
                Column(Modifier.padding(horizontal=12.dp).navigationBarsPadding()) {
                    if(profile!=null) ListItem(headlineContent={Text(ui(R.string.open_external_profile))},leadingContent={Icon(Icons.Outlined.OpenInNew,null)},
                        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {uriHandler.openUri(profile)})
                    ListItem(headlineContent={Text(if(copied) ui(R.string.copied_name) else ui(R.string.copy_name))},leadingContent={Icon(Icons.Outlined.ContentCopy,null)},
                        colors=ListItemDefaults.colors(containerColor=Color.Transparent),modifier=Modifier.clip(RoundedCornerShape(18.dp)).clickable {clipboard.setText(AnnotatedString(author.name));copied=true})
                }
            }
        }
    }
}
