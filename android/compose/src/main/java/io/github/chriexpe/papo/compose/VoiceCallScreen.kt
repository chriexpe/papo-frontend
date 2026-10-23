@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource

/** Full-screen presentation only. Back minimizes; only Leave ends the preview session. */
@Composable
internal fun VoiceCallScreen(calls: VoiceCallContext, onChat: (ServerState, String) -> Unit) {
    val route = calls.state.screen ?: return
    val server = route.server(calls.servers) ?: return
    val channel = server.channels.firstOrNull { it.id == route.channelId } ?: return
    val session = calls.state.session?.takeIf { it.route == route }
    var focused by
        rememberSaveable(route, session?.sessionId, session?.layout) {
            mutableStateOf<String?>(null)
        }
    var sheet by remember(route, session != null) { mutableStateOf("") }
    val people = previewCallParticipants(server, channel.id, session)
    val configuration = LocalConfiguration.current
    val wide = configuration.screenWidthDp > configuration.screenHeightDp
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(route) {
        focus.clearFocus()
        keyboard?.hide()
    }
    BackHandler {
        if (focused != null) focused = null else calls.controller.minimize()
    }
    CompositionLocalProvider(
        LocalProfileContext provides ProfileContext(server, calls.model),
        LocalMediaOverlayBlocking provides true,
    ) {
        Column(
            Modifier.fillMaxSize()
                .background(MaterialTheme.colorScheme.background)
                .hazeSource(LocalHaze.current)
                .statusBarsPadding()
                .navigationBarsPadding()
                .testTag("voice-call-screen")
        ) {
            CallScreenHeader(
                channel.name,
                server.name,
                calls.controller::minimize,
                onMore = if (session != null) ({ sheet = "layout" }) else null,
                onChat = if (wide) ({ onChat(server, channel.id) }) else null,
            )
            if (!wide)
                VoiceChannelTabs(chat = false, onChat = { onChat(server, channel.id) }, onCall = {})
            Text(
                ui(R.string.call_demo_description),
                Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            if (session == null) {
                CallLobby(people, Modifier.weight(1f))
                Column(
                    Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        ui(R.string.call_join_safety),
                        style = MaterialTheme.typography.bodySmall,
                        textAlign = TextAlign.Center,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(12.dp))
                    Button(
                        onClick = { calls.controller.join(route, calls.servers) },
                        modifier = Modifier.heightIn(min = 56.dp).testTag("call-join"),
                        contentPadding = PaddingValues(horizontal = 32.dp),
                    ) {
                        Icon(Icons.Outlined.Headphones, null)
                        Spacer(Modifier.width(10.dp))
                        Text(ui(R.string.call_preview_join))
                    }
                }
            } else {
                CallStage(people, session, focused, { focused = it }, Modifier.weight(1f))
                CallDock(session, calls.controller) { sheet = "audio" }
            }
        }
        if (sheet.isNotEmpty()) CallOptionsSheet(sheet, session, calls.controller) { sheet = "" }
    }
}

@Composable
private fun CallScreenHeader(
    name: String,
    server: String,
    onMinimize: () -> Unit,
    onMore: (() -> Unit)?,
    onChat: (() -> Unit)?,
) {
    GlassPill(
        Modifier.padding(horizontal = LocalUiDimensions.current.floatingPadding, vertical = 8.dp)
            .fillMaxWidth()
    ) {
        Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onMinimize, modifier = Modifier.testTag("call-minimize")) {
                Icon(Icons.Outlined.ExpandMore, ui(R.string.call_minimize))
            }
            Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    name,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    server,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (onChat != null) TextButton(onClick = onChat) { Text(ui(R.string.call_chat_tab)) }
            if (onMore != null)
                IconButton(onClick = onMore, modifier = Modifier.testTag("call-options")) {
                    Icon(Icons.Outlined.MoreHoriz, ui(R.string.call_preview_layout))
                }
            else Spacer(Modifier.size(48.dp))
        }
    }
}

@Composable
private fun CallLobby(people: List<CallParticipant>, modifier: Modifier) {
    Column(
        modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(
            Icons.AutoMirrored.Outlined.VolumeUp,
            null,
            Modifier.size(42.dp),
            tint = MaterialTheme.colorScheme.primary,
        )
        Spacer(Modifier.height(16.dp))
        Text(
            uiQuantity(R.plurals.call_people_in_room, people.size),
            style = MaterialTheme.typography.titleLarge,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        people.forEach { person ->
            Row(
                Modifier.widthIn(max = 360.dp).fillMaxWidth().padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                UserAvatar(person.id, person.name, Modifier.size(44.dp), RoundedCornerShape(16.dp))
                Text(
                    person.name,
                    Modifier.weight(1f),
                    color = userNameColor(person.id),
                    style = MaterialTheme.typography.bodyLarge,
                )
                if (person.muted)
                    Icon(
                        Icons.Outlined.MicOff,
                        ui(R.string.call_muted),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
            }
        }
    }
}
