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
import kotlinx.coroutines.launch
import org.json.JSONArray

class MainActivity : ComponentActivity() {
    private val model: PapoViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { PapoApp(model) }
    }
    override fun onStart() { super.onStart(); model.resume() }
}

private val Blue = Color(0xFF0A84FF)
internal val LocalGlass = compositionLocalOf { true }
internal val LocalHaze = compositionLocalOf<HazeState> { error("Missing content layer") }

@Composable
fun PapoApp(model: PapoViewModel = viewModel()) {
    val state by model.state.collectAsStateWithLifecycle()
    val preferences = LocalContext.current.getSharedPreferences("compose-appearance", 0)
    var theme by rememberSaveable { mutableStateOf(preferences.getString("theme", "System")!!) }
    var glass by rememberSaveable { mutableStateOf(preferences.getBoolean("glass", true)) }
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
        CompositionLocalProvider(LocalHaze provides haze, LocalGlass provides glass, LocalContentColor provides colors.onSurface) {
            var adding by rememberSaveable { mutableStateOf(false) }
            var tab by rememberSaveable { mutableStateOf("Chats") }
            var inConversation by rememberSaveable(state.activeUrl) { mutableStateOf(false) }
            val server = state.active
            var profileId by remember(state.activeUrl) {mutableStateOf<String?>(null)}
            var externalProfile by remember(state.activeUrl) {mutableStateOf<io.github.chriexpe.papo.core.ExternalAuthorView?>(null)}
            val openProfile:(String)->Unit = {externalProfile=null;profileId=it}
            val leaveConversation:()->Unit = { inConversation = false; tab = "Chats" }
            CompositionLocalProvider(LocalProfileContext provides server?.let {ProfileContext(it,model)},LocalOpenProfile provides openProfile,LocalOpenExternalProfile provides {author ->profileId=null;externalProfile=author},LocalMediaOverlayBlocking provides (profileId!=null || externalProfile!=null)) {
            // Child dialogs, sheets and the IME consume Back first. Keep the
            // activity alive until navigation reaches the conversation list.
            BackHandler(inConversation || tab != "Chats" || profileId != null || externalProfile != null || adding) {
                when {
                    adding -> adding = false
                    profileId != null -> profileId = null
                    externalProfile != null -> externalProfile = null
                    inConversation -> leaveConversation()
                    else -> tab = "Chats"
                }
            }
            Box(Modifier.fillMaxSize().background(colors.background)) {
                if (state.servers.isEmpty()) {
                    Welcome(state.opening, state.error, model::clearError, model::exploreDemo) { model.addServer(it) }
                } else if (server == null) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                } else if (server.screen != "chat" && tab != "Servers") {
                    Authentication(server, state.error, onBack = { tab = "Servers" },
                        onSwitch = { adding = true }, command = { model.command(it) })
                } else if (inConversation && server.conversation != null) {
                    ConversationScreen(server, model, onBack = leaveConversation)
                } else {
                    Box(Modifier.fillMaxSize()) {
                        when (tab) {
                            "Chats" -> ConversationList(server, onServer = { tab = "Servers" },
                                onOpen = { id -> model.command(Action.SelectChannel(id)); inConversation = true },
                                onProfile = openProfile)
                            "Servers" -> ServerList(state, onAdd = { adding = true }, onSelect = { model.activate(it); tab = "Chats" })
                            else -> ClientSettings(server, theme, glass,
                                onTheme = { theme = it; preferences.edit().putString("theme", it).apply() },
                                onGlass = { glass = it; preferences.edit().putBoolean("glass", it).apply() },
                                onLogout = { if (server.isDemo) model.leaveDemo() else model.command(Action.Logout) })
                        }
                        FloatingNavigation(tab, onSelect = { tab = it }, Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(16.dp))
                    }
                }
                val error = if (state.servers.isEmpty()) "" else state.error.ifBlank { server?.error.orEmpty() }
                if (error.isNotBlank() && server?.screen == "chat") {
                    FeedbackPill(error, model::clearError, Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(12.dp))
                }
            }
            if(server?.screen=="chat") externalProfile?.let {author ->ExternalProfileSheet(server,author) {externalProfile=null}}
            if(server?.screen=="chat") profileId?.let {id -> UserProfileSheet(server,model,id,onDismiss={profileId=null},onMessage={user ->profileId=null;model.command(Action.OpenDm(user),server.url) {inConversation=true}})}
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
    GlassPill(modifier.widthIn(max = 400.dp)) {
        Row(Modifier.padding(6.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            listOf("Chats" to Icons.Outlined.Forum, "Servers" to Icons.Outlined.Dns, "Settings" to Icons.Outlined.Tune).forEach { (name, icon) ->
                val active = selected == name
                Column(Modifier.weight(1f).clip(RoundedCornerShape(25.dp)).background(if (active) MaterialTheme.colorScheme.primaryContainer else Color.Transparent)
                    .clickable(onClickLabel = name) { onSelect(name) }.padding(horizontal = 12.dp, vertical = 10.dp),
                    horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Icon(icon, null, Modifier.size(22.dp), tint = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(name, style = MaterialTheme.typography.labelSmall, fontWeight = if (active) FontWeight.Bold else FontWeight.Normal)
                }
            }
        }
    }
}

@Composable
private fun ConversationList(server: ServerState, onServer: () -> Unit, onOpen: (String) -> Unit, onProfile: (String) -> Unit) {
    var section by rememberSaveable { mutableStateOf("Channels") }
    var query by rememberSaveable { mutableStateOf("") }
    Column(Modifier.fillMaxSize().hazeSource(LocalHaze.current).statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp, top = 20.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(if (server.isDemo) "DEMO SPACE" else "YOUR SPACE", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, letterSpacing = 1.5.sp)
                Text(server.name, style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            IconButton(onClick = onServer) { Icon(Icons.Outlined.ExpandMore, "Switch server") }
        }
        if (server.connection != "online") ConnectionBanner(server.connection) {}
        OutlinedTextField(query, { query = it }, placeholder = { Text("Find a conversation") }, leadingIcon = { Icon(Icons.Outlined.Search, null) }, singleLine = true,
            shape = RoundedCornerShape(24.dp), modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp))
        Row(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("Channels", "Direct", "People").forEach { label -> FilterChip(section == label, { section = label }, label = { Text(label) }) }
        }
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(start = 12.dp, end = 12.dp, bottom = 128.dp)) {
            val channels = if (section == "Direct") server.dms else server.channels.filter { it.kind != "direct" && it.kind != "voice" }
            if (section == "People") {
                val people = server.members.filter { it.id != server.me && it.name.contains(query, true) }
                if (people.isEmpty()) item { EmptyState("No people here yet", "People in your community will appear here.") }
                items(people, key = { it.id }) { person -> ListItem(
                    headlineContent = { Text(person.name, fontWeight = FontWeight.Medium) }, supportingContent = { Text(person.presence.replaceFirstChar(Char::uppercase)) },
                    leadingContent = { UserAvatar(person.id,person.name) }, trailingContent = { Icon(Icons.Outlined.ChatBubbleOutline, null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent), modifier = Modifier.clip(RoundedCornerShape(18.dp)).clickable { onProfile(person.id) }) }
            } else {
                val filtered = channels.filter { it.name.contains(query, true) }
                if (filtered.isEmpty()) item { EmptyState(if (query.isNotBlank()) "No conversations found" else if (section == "Direct") "A conversation starts with hello." else "A little quiet here.",
                    if (section == "Direct") "Find someone in People to start a direct message." else "Your text channels will appear here.") }
                items(filtered, key = { it.id }) { channel ->
                    if (channel.kind == "category") Text(channel.name.uppercase(), style = MaterialTheme.typography.labelSmall, letterSpacing = 1.sp, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 20.dp, top = 24.dp, bottom = 8.dp))
                    else ListItem(
                        headlineContent = { Text(channel.name, fontWeight = if (channel.unread) FontWeight.Bold else FontWeight.Medium) },
                        supportingContent = if (channel.topic.isNotBlank()) ({ Text(channel.topic, maxLines = 1, overflow = TextOverflow.Ellipsis) }) else null,
                        leadingContent = { if (section == "Direct") UserAvatar(channel.userId,channel.name,Modifier.clip(CircleShape).clickable {onProfile(channel.userId)}) else Box(Modifier.size(44.dp).clip(RoundedCornerShape(14.dp)).background(MaterialTheme.colorScheme.surfaceContainer), contentAlignment = Alignment.Center) { Text("#", fontSize = 23.sp, color = MaterialTheme.colorScheme.onSurfaceVariant) } },
                        trailingContent = { if (channel.mentions > 0) Badge { Text(channel.mentions.toString()) } else if (channel.unread) Box(Modifier.size(8.dp).background(MaterialTheme.colorScheme.primary, CircleShape)) else Icon(Icons.Outlined.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant) },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent), modifier = Modifier.clip(RoundedCornerShape(18.dp)).clickable { onOpen(channel.id) })
                }
            }
        }
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
private fun ServerList(state: PapoState, onAdd: () -> Unit, onSelect: (String) -> Unit) {
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).statusBarsPadding(), contentPadding = PaddingValues(24.dp, 24.dp, 24.dp, 140.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item { Text("Your servers", style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.SemiBold) }
        item { Text("Different places. The same you.", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(bottom = 16.dp)) }
        items(state.servers, key = { it.url }) { server ->
            ListItem(headlineContent = { Text(server.name, fontWeight = FontWeight.SemiBold) }, supportingContent = { Text(server.url, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                leadingContent = { Avatar(server.name) }, trailingContent = { Icon(if (state.activeUrl == server.url) Icons.Outlined.CheckCircle else Icons.Outlined.ChevronRight, null) },
                modifier = Modifier.clip(RoundedCornerShape(22.dp)).clickable { onSelect(server.url) })
        }
        item { OutlinedButton(onClick = onAdd, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) { Icon(Icons.Outlined.Add, null); Spacer(Modifier.width(8.dp)); Text("Add server") } }
    }
}

@Composable
private fun ClientSettings(server: ServerState, theme: String, glass: Boolean, onTheme: (String) -> Unit, onGlass: (Boolean) -> Unit, onLogout: () -> Unit) {
    val openProfile=LocalOpenProfile.current
    var logout by remember { mutableStateOf(false) }
    LazyColumn(Modifier.fillMaxSize().hazeSource(LocalHaze.current).statusBarsPadding(), contentPadding = PaddingValues(24.dp, 24.dp, 24.dp, 140.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        item { Text("Settings", style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.SemiBold) }
        item { SettingsGroup("YOUR ACCOUNT") { ListItem(headlineContent = { Text(server.myName, fontWeight = FontWeight.SemiBold) }, supportingContent = { Text(server.name) }, leadingContent = { UserAvatar(server.me,server.myName) },modifier=Modifier.clickable {openProfile(server.me)}) } }
        item { SettingsGroup("APPEARANCE") {
            Column(Modifier.padding(16.dp)) {
                Text("Theme", style = MaterialTheme.typography.titleMedium)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { listOf("System", "Light", "Dark").forEach { choice -> FilterChip(theme == choice, { onTheme(choice) }, label = { Text(choice) }) } }
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = .4f))
            ListItem(headlineContent = { Text("Frosted controls") }, supportingContent = { Text("Glass on floating bars and pills") }, trailingContent = { Switch(glass, onGlass, Modifier.semantics { contentDescription = "Frosted controls" }) })
        } }
        item { SettingsGroup("ABOUT") { ListItem(headlineContent = { Text("Papo for Android") }, supportingContent = { Text("Native Compose · ${BuildConfig.VERSION_NAME}") }, leadingContent = { Icon(Icons.Outlined.Forum, null) }) } }
        item { TextButton(onClick = { if (server.isDemo) onLogout() else logout = true }) { Icon(Icons.Outlined.Logout, null); Spacer(Modifier.width(8.dp)); Text(if (server.isDemo) "Leave demo" else "Sign out of ${server.name}") } }
    }
    if (logout) AlertDialog(onDismissRequest = { logout = false }, title = { Text("Sign out?") }, text = { Text("You’ll need to sign in again on ${server.name}.") }, confirmButton = { TextButton(onClick = { logout = false; onLogout() }) { Text("Sign out") } }, dismissButton = { TextButton(onClick = { logout = false }) { Text("Cancel") } })
}

@Composable
private fun SettingsGroup(label: String, content: @Composable ColumnScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(label, style = MaterialTheme.typography.labelSmall, letterSpacing = 1.sp, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 12.dp))
        Column(Modifier.clip(RoundedCornerShape(22.dp)).background(MaterialTheme.colorScheme.surface), content = content)
    }
}

@Composable
internal fun Avatar(name: String, modifier: Modifier = Modifier) {
    val colors = listOf(Color(0xFF305C88), Color(0xFF76503C), Color(0xFF5B537E), Color(0xFF386858))
    val initials = name.split(' ').filter { it.isNotBlank() }.take(2).map { it.first() }.joinToString("").uppercase()
    Box(modifier.size(40.dp).clip(CircleShape).background(colors[(name.hashCode() and Int.MAX_VALUE) % colors.size]), contentAlignment = Alignment.Center) {
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
