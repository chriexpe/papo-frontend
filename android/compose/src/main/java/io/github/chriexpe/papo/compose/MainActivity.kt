@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import io.github.chriexpe.papo.core.Action
import android.os.Bundle
import android.app.Activity
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.chrisbanes.haze.HazeState
import dev.chrisbanes.haze.HazeStyle
import dev.chrisbanes.haze.HazeTint
import dev.chrisbanes.haze.hazeEffect
import dev.chrisbanes.haze.hazeSource
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import org.json.JSONArray

class MainActivity : ComponentActivity() {
    private val model: PapoViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if(savedInstanceState==null) MessageAlerts.target(intent)?.let(model::offerNotification)
        setContent { PapoApp(model) }
    }
    override fun onStart() { super.onStart();MessageAlerts.foreground=true;MessageAlerts.refresh();model.resume() }
    override fun onStop() {MessageAlerts.foreground=false;super.onStop()}
    // Notification routes are consumed immediately. Keep the launch intent so
    // recreating the Activity cannot replay a later navigation request.
    override fun onNewIntent(intent:android.content.Intent) {super.onNewIntent(intent);MessageAlerts.target(intent)?.let(model::offerNotification)}
}

private val Blue = Color(0xFF0A84FF)
internal val LocalGlass = compositionLocalOf { true }
internal val LocalHaze = compositionLocalOf<HazeState> { error("Missing content layer") }

@Composable
fun PapoApp(model: PapoViewModel = viewModel()) {
    val state by model.state.collectAsStateWithLifecycle()
    val preferences = LocalContext.current.getSharedPreferences("compose-appearance", 0)
    val appearance=remember(preferences) {ClientPreferences(preferences)}
    DisposableEffect(appearance) {onDispose {appearance.close()}}
    val theme=appearance.theme
    val glass=appearance.glass
    val dark = when (theme) { "Dark" -> true; "Light" -> false; else -> isSystemInDarkTheme() }
    val view = LocalView.current
    SideEffect {
        (view.context as? Activity)?.window?.let { window ->
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    val colors = if (dark) darkColorScheme(
        primary = Color(0xFF6BB1FF), onPrimary = Color(0xFF001D36), primaryContainer = Color(0xFF123A62),
        secondaryContainer = Color(0xFF123A62), onSecondaryContainer = Color(0xFFBEDDFF),
        background = Color(0xFF161618), surface = Color(0xFF1F1F22), surfaceContainer = Color(0xFF29292E),
        surfaceContainerLow = Color(0xFF1F1F22), surfaceContainerHigh = Color(0xFF2C2C30), surfaceContainerHighest = Color(0xFF323236),
        outline = Color(0xFF85858F), outlineVariant = Color(0xFF3C3C44),
        onBackground = Color(0xFFF4F4F7), onSurface = Color(0xFFF4F4F7), onSurfaceVariant = Color(0xFFB9BBC4),
    ) else lightColorScheme(
        primary = Color(0xFF0060C5), primaryContainer = Color(0xFFD9E9FF),
        secondaryContainer = Color(0xFFD9E9FF), onSecondaryContainer = Color(0xFF00325A),
        background = Color(0xFFF9FAFC), surface = Color(0xFFF4F4F7), surfaceContainer = Color(0xFFECEEF3),
        surfaceContainerLow = Color(0xFFF4F4F7), surfaceContainerHigh = Color(0xFFE6E8ED), surfaceContainerHighest = Color(0xFFDEE1E7),
        outline = Color(0xFF777C85), outlineVariant = Color(0xFFD9DCE3),
        onBackground = Color(0xFF191B21), onSurface = Color(0xFF191B21), onSurfaceVariant = Color(0xFF555D69),
    )
    MaterialTheme(colorScheme = colors) {
        val haze = remember { HazeState() }
        CompositionLocalProvider(LocalHaze provides haze, LocalClientPreferences provides appearance, LocalGlass provides glass, LocalContentColor provides colors.onSurface) {
            var adding by rememberSaveable { mutableStateOf(false) }
            var tab by rememberSaveable { mutableStateOf("Chats") }
            var settingsOrigin by rememberSaveable {mutableStateOf("Chats")}
            var inConversation by rememberSaveable { mutableStateOf(false) }
            val server = state.active
            val conversationStates=rememberSaveableStateHolder()
            var conversationScope by rememberSaveable {mutableStateOf("")}
            val showingConversation=inConversation && conversationScope=="${server?.url}/${server?.me}"
            fun enterConversation(target:ServerState) {conversationScope="${target.url}/${target.me}";inConversation=true}
            var profileId by remember {mutableStateOf<String?>(null)}
            var profileServerUrl by remember {mutableStateOf("")}
            var profileOwner by remember {mutableStateOf("")}
            val openScopedProfile:(ServerState,String)->Unit={target,id ->profileServerUrl=target.url;profileOwner=target.me;profileId=id}
            val profileServer=state.servers.firstOrNull {it.url==profileServerUrl && it.me==profileOwner && it.screen=="chat"}
            var externalProfile by remember(state.activeUrl) {mutableStateOf<io.github.chriexpe.papo.core.ExternalAuthorView?>(null)}
            val openProfile:(String)->Unit = {id ->externalProfile=null;server?.let {openScopedProfile(it,id)}}
            val leaveConversation:()->Unit = { inConversation = false }
            SideEffect {model.visibleConversation(if(showingConversation && profileId==null && externalProfile==null) server?.url.orEmpty() else "")}
            val notificationTarget by model.notificationTarget.collectAsStateWithLifecycle()
            LaunchedEffect(notificationTarget) {
                val target=notificationTarget ?: return@LaunchedEffect
                val matches=kotlinx.coroutines.withTimeoutOrNull(20_000) {
                    model.state.map {current ->current.servers.filter {candidate ->candidate.screen=="chat" && candidate.me.isNotBlank() &&
                        (target.server.isBlank() || target.server==candidate.url) && (target.owner.isBlank() || target.owner==candidate.me) &&
                        (candidate.channels+candidate.dms).any {it.id==target.channel}}}.first {it.isNotEmpty()}
                }.orEmpty()
                if(matches.size==1) {
                    val destination=matches.single()
                    model.consumeNotification(target)
                    model.navigate(destination,Action.SelectChannel(target.channel),target.message) {
                        adding=false;profileId=null;externalProfile=null;tab="Chats";enterConversation(destination)
                    }
                } else model.consumeNotification(target,"This notification no longer matches a signed-in conversation.")
            }

            CompositionLocalProvider(LocalProfileContext provides server?.let {ProfileContext(it,model)},LocalOpenProfile provides openProfile,LocalOpenExternalProfile provides {author ->profileId=null;externalProfile=author},LocalMediaOverlayBlocking provides (profileId!=null || externalProfile!=null)) {
            // Child dialogs, sheets and the IME consume Back first. Keep the
            // activity alive until navigation reaches the conversation list.
            BackHandler(showingConversation || tab != "Chats" || profileId != null || externalProfile != null || adding) {
                when {
                    adding -> adding = false
                    profileId != null -> profileId = null
                    externalProfile != null -> externalProfile = null
                    showingConversation -> leaveConversation()
                    tab=="GlobalDirect" || tab=="GlobalNotifications" -> tab="Servers"
                    tab == "Settings" -> tab = settingsOrigin
                    else -> tab = "Chats"
                }
            }
            Box(Modifier.fillMaxSize().background(colors.background)) {
                if (state.servers.isEmpty()) {
                    Welcome(state.opening, state.error, model::clearError, model::exploreDemo) { model.addServer(it) }
                } else if (server == null) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                } else if (server.screen != "chat" && tab !in listOf("Servers","GlobalDirect","GlobalNotifications")) {
                    Authentication(server, state.error, onBack = { tab = "Servers" },
                        onSwitch = { adding = true }, command = { model.command(it) })
                } else if (showingConversation && server.conversation != null) {
                    ConversationScreen(server, model, onBack = leaveConversation)
                } else {
                    Box(Modifier.fillMaxSize()) {
                        when (tab) {
                            "Chats", "Direct" -> conversationStates.SaveableStateProvider("${server.url}/${server.me}/$tab") {ConversationList(server, model, appError = state.error, direct = tab=="Direct", onSettings = {settingsOrigin=tab;tab="Settings"},
                                onOpen = { id -> model.navigate(server,Action.SelectChannel(id)) {enterConversation(server)} },
                                onProfile = openProfile,
                                onMessage = { channelId,messageId ->model.navigate(server,Action.SelectChannel(channelId),messageId) {enterConversation(server)}})}
                            "Servers" -> ServerList(state, model, onAdd = { adding = true }, onSelect = { inConversation=false;model.activate(it); tab = "Chats" },onDirect={tab="GlobalDirect"},onNotifications={tab="GlobalNotifications"})
                            "GlobalDirect" -> conversationStates.SaveableStateProvider("global/direct") {
                                GlobalDirectMessages(state,model,onBack={tab="Servers"},onProfile=openScopedProfile,onOpen={target,channel ->model.navigate(target,Action.SelectChannel(channel.id)) {enterConversation(target)}})
                            }
                            "GlobalNotifications" -> conversationStates.SaveableStateProvider("global/notifications") {
                                GlobalNotifications(state,model,onBack={tab="Servers"},onOpen={target,item ->model.navigate(target,Action.SelectChannel(item.channelId),item.messageId.takeIf {it.isNotBlank()}) {enterConversation(target)}})
                            }
                            else -> ClientSettings(server, model, onBack={tab=settingsOrigin},
                                onLogout = { if (server.isDemo) model.leaveDemo() else model.command(Action.Logout) })
                        }
                        if(tab!="Settings") FloatingNavigation(if(tab=="GlobalDirect" || tab=="GlobalNotifications") "Servers" else tab, onSelect = { tab = it }, Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(16.dp))
                    }
                }
                val error = if (state.servers.isEmpty()) "" else state.error.ifBlank { server?.error.orEmpty() }
                if (error.isNotBlank() && server?.screen == "chat") {
                    FeedbackPill(error, model::clearError, Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(12.dp))
                }
            }
            if(server?.screen=="chat") externalProfile?.let {author ->ExternalProfileSheet(server,author) {externalProfile=null}}
            profileServer?.let {target ->profileId?.let {id -> CompositionLocalProvider(LocalProfileContext provides ProfileContext(target,model)) {
                UserProfileSheet(target,model,id,onDismiss={profileId=null},onMessage={user ->profileId=null;model.navigate(target,Action.OpenDm(user)) {enterConversation(target)}})
            }}}
            }
            if (adding) ModalBottomSheet(onDismissRequest = { adding = false }) {
                AddServerForm(state.opening, state.error, modifier = Modifier.padding(horizontal = 24.dp).navigationBarsPadding()) {
                    model.addServer(it) { adding = false }
                }
                Spacer(Modifier.height(24.dp))
            }
        }
    }
}

/** One content capture, sampled only behind floating controls. */
@Composable
internal fun GlassPill(modifier: Modifier = Modifier, radius: Int = 32, content: @Composable () -> Unit) {
    val color = MaterialTheme.colorScheme.surface
    val glass = LocalGlass.current
    val shape = RoundedCornerShape(radius.dp)
    val haze = LocalHaze.current
    Box(modifier.clip(shape).then(
        if (glass) Modifier.hazeEffect(haze, style = HazeStyle(
            backgroundColor = MaterialTheme.colorScheme.background,
            tint = HazeTint(color.copy(alpha = 0.78f)), blurRadius = 24.dp, noiseFactor = 0.035f,
            fallbackTint = HazeTint(color),
        )) else Modifier.background(color)
    ).border(BorderStroke(0.7.dp, MaterialTheme.colorScheme.onSurface.copy(alpha = 0.12f)), shape)) { content() }
}

@Composable
private fun Welcome(opening: Boolean, error: String, clearError: () -> Unit, onDemo: () -> Unit, onConnect: (String) -> Unit) {
    Box(Modifier.fillMaxSize().imePadding()) {
        Box(Modifier.fillMaxSize().hazeSource(LocalHaze.current)) {
            Canvas(Modifier.fillMaxSize()) {
                drawCircle(Brush.radialGradient(listOf(Blue.copy(alpha = 0.14f), Color.Transparent), center = Offset(size.width * .8f, size.height * .18f), radius = size.width * .8f), radius = size.width, center = Offset(size.width * .8f, size.height * .18f))
            }
        }
        Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(horizontal = 28.dp).verticalScrollCompat(),
            verticalArrangement = Arrangement.Center) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                BrandMark()
                Text("papo", fontSize = 28.sp, fontWeight = FontWeight.Bold, letterSpacing = (-1).sp)
            }
            Spacer(Modifier.height(52.dp))
            Text("Good conversations\nstart here.", fontSize = 38.sp, lineHeight = 43.sp, letterSpacing = (-1.5).sp, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(16.dp))
            Text("A place for your people. Connect to a Papo server to make yourself at home.",
                style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(40.dp))
            AddServerForm(opening, error, showHeading = false, onConnect = onConnect)
            TextButton(onClick = onDemo, Modifier.align(Alignment.CenterHorizontally)) { Text("Explore demo") }
            if (error.isNotBlank()) TextButton(onClick = clearError) { Text("Dismiss error") }
            Spacer(Modifier.height(40.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.Lock, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text("Your server. Your community.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun BrandMark() {
    Box(Modifier.size(46.dp).clip(RoundedCornerShape(15.dp)).background(Blue), contentAlignment = Alignment.Center) {
        Icon(Icons.Outlined.Forum, null, Modifier.size(26.dp), Color.White)
    }
}

@Composable
private fun AddServerForm(opening: Boolean, error: String, modifier: Modifier = Modifier, showHeading: Boolean = true, onConnect: (String) -> Unit) {
    var address by rememberSaveable { mutableStateOf("") }
    val keyboard = LocalSoftwareKeyboardController.current
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (showHeading) {
            Text("Add a server", style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.SemiBold)
            Text("Enter the address shared by your community.", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        OutlinedTextField(address, { address = it }, label = { Text("Server address") },
            supportingText = { Text("Use the address shared by your community.") },
            leadingIcon = { Icon(Icons.Outlined.Dns, null) }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { keyboard?.hide() }),
            shape = RoundedCornerShape(18.dp), modifier = Modifier.fillMaxWidth(), enabled = !opening)
        if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
        Button(onClick = { onConnect(address.trim()) }, enabled = address.isNotBlank() && !opening,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp), contentPadding = PaddingValues(16.dp)) {
            if (opening) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            else { Icon(Icons.Outlined.Add, null, Modifier.size(20.dp)); Spacer(Modifier.width(8.dp)); Text("Add server") }
        }
    }
}

@Composable
private fun Authentication(server: ServerState, appError: String, onBack: () -> Unit, onSwitch: () -> Unit, command: (Action) -> Unit) {
    var username by rememberSaveable(server.url) { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var register by rememberSaveable(server.url) { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().imePadding().statusBarsPadding().navigationBarsPadding().padding(24.dp).verticalScrollCompat(), verticalArrangement = Arrangement.Center) {
        BrandMark()
        Spacer(Modifier.height(32.dp))
        Text(if (server.locked) "A little privacy." else if (server.screen == "needs_server") "Make it yours." else "Welcome to\n${server.name}",
            style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.SemiBold)
        Text(server.url, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(vertical = 12.dp))
        if (server.screen == "starting") {
            LinearProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 20.dp))
            Text("Connecting to your server…", color = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            if (!server.locked) OutlinedTextField(username, { username = it }, label = { Text(if (server.screen == "needs_server") "Server name" else "Username") }, singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !server.busy, shape = RoundedCornerShape(16.dp))
            if (server.screen != "needs_server") OutlinedTextField(password, { password = it }, label = { Text(if (server.locked) "Server password" else "Password") }, singleLine = true, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth().padding(top = 12.dp), enabled = !server.busy, shape = RoundedCornerShape(16.dp))
            val error = appError.ifBlank { server.error }
            if (error.isNotBlank()) Text(error, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(vertical = 12.dp))
            Button(onClick = {
                when {
                    server.locked -> command(Action.Unlock(password))
                    server.screen == "needs_server" -> command(Action.CreateServer(username))
                    else -> command(if (register) Action.Register(username, password) else Action.Login(username, password))
                }
                password = ""
            }, enabled = !server.busy && (if (server.locked) password.isNotBlank() else if (server.screen == "needs_server") username.isNotBlank() else username.isNotBlank() && password.isNotBlank()), modifier = Modifier.fillMaxWidth().padding(top = 24.dp).heightIn(min = 52.dp)) {
                if (server.busy) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                else Text(if (server.locked) "Unlock server" else if (server.screen == "needs_server") "Create server" else if (register) "Create account" else "Sign in")
            }
            if (!server.locked && server.screen == "auth") TextButton(onClick = { register = !register }, Modifier.align(Alignment.CenterHorizontally)) { Text(if (register) "Already have an account? Sign in" else "New here? Create an account") }
        }
        TextButton(onClick = onSwitch, Modifier.align(Alignment.CenterHorizontally).padding(top = 20.dp)) { Text("Connect to another server") }
        TextButton(onClick = onBack, Modifier.align(Alignment.CenterHorizontally)) { Text("Your servers") }
    }
}

@Composable
private fun FloatingNavigation(selected: String, onSelect: (String) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier.widthIn(max=400.dp).fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(12.dp),verticalAlignment=Alignment.CenterVertically) {
        GlassPill(Modifier.weight(1f).testTag("chat-navigation")) {
            Row(Modifier.padding(6.dp),horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                NavigationDestination("Chats",Icons.Outlined.Forum,selected=="Chats",{onSelect("Chats")},Modifier.weight(1f))
                NavigationDestination("Direct",Icons.Outlined.ChatBubbleOutline,selected=="Direct",{onSelect("Direct")},Modifier.weight(1f))
            }
        }
        GlassPill(Modifier.size(72.dp).testTag("servers-navigation"),radius=24) {
            NavigationDestination("Servers",Icons.Outlined.Dns,selected=="Servers",{onSelect("Servers")},Modifier.fillMaxSize().padding(6.dp),radius=18)
        }
    }
}

@Composable
private fun NavigationDestination(name:String,icon:ImageVector,active:Boolean,onClick:()->Unit,modifier:Modifier=Modifier,radius:Int=25) {
    Column(modifier.clip(RoundedCornerShape(radius.dp)).background(if(active) MaterialTheme.colorScheme.primaryContainer else Color.Transparent)
        .selectable(selected=active,onClick=onClick,role=Role.Tab).heightIn(min=58.dp).padding(horizontal=10.dp,vertical=8.dp),
        horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(3.dp,Alignment.CenterVertically)) {
        Icon(icon,null,Modifier.size(22.dp),tint=if(active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
        Text(name,style=MaterialTheme.typography.labelSmall,fontWeight=if(active) FontWeight.Bold else FontWeight.Normal,maxLines=1)
    }
}

@Composable
internal fun EmptyState(title: String, detail: String) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 48.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(Icons.Outlined.ChatBubbleOutline, null, Modifier.size(36.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(title, style = MaterialTheme.typography.titleLarge)
        Text(detail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun ServerList(state: PapoState, model:PapoViewModel, onAdd: () -> Unit, onSelect: (String) -> Unit,onDirect:()->Unit,onNotifications:()->Unit) {
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).statusBarsPadding(), contentPadding = PaddingValues(24.dp, 24.dp, 24.dp, 140.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Row(Modifier.fillMaxWidth().testTag("global-actions"),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                GlobalAction("Direct messages",Icons.Outlined.ChatBubbleOutline,state.servers.filter {it.screen=="chat"}.sumOf {s ->s.dms.count {it.unread}},onDirect,Modifier.weight(1f))
                GlobalAction("Notifications",Icons.Outlined.NotificationsNone,state.servers.filter {it.screen=="chat"}.sumOf {it.notificationCount.toLong()}.coerceAtMost(Int.MAX_VALUE.toLong()).toInt(),onNotifications,Modifier.weight(1f))
            }
        }
        item {Text("Your servers",style=MaterialTheme.typography.labelLarge,color=MaterialTheme.colorScheme.onSurfaceVariant,modifier=Modifier.padding(top=16.dp,start=8.dp,bottom=4.dp))}
        items(state.servers, key = { it.url }) { server ->
            ListItem(headlineContent = { Text(server.name, fontWeight = FontWeight.SemiBold) }, supportingContent = { Text(server.url, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                leadingContent = { ServerIcon(server,model) }, trailingContent = { Icon(if (state.activeUrl == server.url) Icons.Outlined.CheckCircle else Icons.Outlined.ChevronRight, null) },
                modifier = Modifier.clip(RoundedCornerShape(22.dp)).clickable { onSelect(server.url) })
        }
        item { OutlinedButton(onClick = onAdd, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) { Icon(Icons.Outlined.Add, null); Spacer(Modifier.width(8.dp)); Text("Add server") } }
    }
}

@Composable
internal fun Avatar(name: String, modifier: Modifier = Modifier, shape:androidx.compose.ui.graphics.Shape = CircleShape) {
    val colors = listOf(Color(0xFF305C88), Color(0xFF76503C), Color(0xFF5B537E), Color(0xFF386858))
    val initials = name.split(' ').filter { it.isNotBlank() }.take(2).map { it.first() }.joinToString("").uppercase()
    Box(modifier.size(40.dp).clip(shape).background(colors[(name.hashCode() and Int.MAX_VALUE) % colors.size]), contentAlignment = Alignment.Center) {
        Text(initials, color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
internal fun ConnectionBanner(connection: String, onRetry: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.Outlined.CloudOff, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(if (connection == "connecting") "Reconnecting…" else "Offline · saved messages are available", style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f).padding(horizontal = 8.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        TextButton(onClick = onRetry) { Text("Retry") }
    }
}

@Composable
private fun Modifier.verticalScrollCompat(): Modifier = this.then(Modifier.verticalScroll(androidx.compose.foundation.rememberScrollState()))

/** All app-wide failures share one material, spacing, icon and dismissal. */
@Composable
private fun FeedbackPill(message: String, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    GlassPill(modifier.fillMaxWidth().widthIn(max = 520.dp), radius = 24) {
        Row(Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(Icons.Outlined.ErrorOutline, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.error)
            Text(message, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f), color = MaterialTheme.colorScheme.onSurface)
            IconButton(onClick = onDismiss) { Icon(Icons.Outlined.Close, "Dismiss notification", Modifier.size(20.dp)) }
        }
    }
}
