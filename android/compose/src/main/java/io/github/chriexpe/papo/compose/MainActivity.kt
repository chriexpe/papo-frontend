@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.app.Activity
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.chrisbanes.haze.HazeState
import io.github.chriexpe.papo.core.Action
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

class MainActivity : ComponentActivity() {
    private val model: PapoViewModel by viewModels()

    override fun attachBaseContext(base: android.content.Context) {
        super.attachBaseContext(AppLanguage.context(base))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        AppLanguage.update(this)
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (savedInstanceState == null) MessageAlerts.target(intent)?.let(model::offerNotification)
        consumeRecovery(intent)
        setContent { PapoApp(model) }
    }

    override fun onStart() {
        super.onStart()
        MessageAlerts.foreground = true
        MessageAlerts.refresh()
        model.resume()
    }

    override fun onStop() {
        MessageAlerts.foreground = false
        super.onStop()
    }

    // Notification routes are consumed immediately. Keep the launch intent so
    // recreating the Activity cannot replay a later navigation request.
    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        MessageAlerts.target(intent)?.let(model::offerNotification)
        consumeRecovery(intent)
    }

    private fun consumeRecovery(intent: android.content.Intent?) {
        val raw = intent?.dataString ?: return
        if (isRecoveryIntent(raw)) {
            model.offerRecovery(raw)
            intent.data = null
        }
    }
}

@Composable
fun PapoApp(model: PapoViewModel = viewModel()) {
    val context = LocalContext.current
    val localeConfiguration = androidx.compose.ui.platform.LocalConfiguration.current
    LaunchedEffect(localeConfiguration) { AppLanguage.update(context) }
    val state by model.state.collectAsStateWithLifecycle()
    val preferences = LocalContext.current.getSharedPreferences("compose-appearance", 0)
    val appearance = remember(preferences) { ClientPreferences(preferences) }
    DisposableEffect(appearance) { onDispose { appearance.close() } }
    val theme = appearance.theme
    val glass = appearance.glass
    val messageAppearance = appearance.messageAppearance(state.active)
    val accountOwners = state.servers.map { Triple(it.url, it.me, it.screen) }
    LaunchedEffect(accountOwners) { model.accountAppearance.synchronize(state.servers) }
    val appearanceServer = state.active
    val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
    // Refresh on account/reconnect/app entry. The request itself belongs to the ViewModel.
    LaunchedEffect(
        appearanceServer?.url,
        appearanceServer?.me,
        appearanceServer?.screen,
        appearanceServer?.connection,
        lifecycle,
    ) {
        if (appearanceServer?.screen == "chat" && appearanceServer.me.isNotBlank()) {
            lifecycle.repeatOnLifecycle(androidx.lifecycle.Lifecycle.State.STARTED) {
                if (appearanceServer.connection == "online")
                    model.accountAppearance.refresh(appearanceServer, force = true)
                kotlinx.coroutines.awaitCancellation()
            }
        }
    }
    val dark =
        when (theme) {
            "Dark" -> true
            "Light" -> false
            else -> isSystemInDarkTheme()
        }
    val view = LocalView.current
    SideEffect {
        (view.context as? Activity)?.window?.let { window ->
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    val colors =
        if (dark)
            darkColorScheme(
                primary = Color(0xFF6BB1FF),
                onPrimary = Color(0xFF001D36),
                primaryContainer = Color(0xFF123A62),
                secondaryContainer = Color(0xFF123A62),
                onSecondaryContainer = Color(0xFFBEDDFF),
                background = Color(0xFF161618),
                surface = Color(0xFF1F1F22),
                surfaceContainer = Color(0xFF29292E),
                surfaceContainerLow = Color(0xFF1F1F22),
                surfaceContainerHigh = Color(0xFF2C2C30),
                surfaceContainerHighest = Color(0xFF323236),
                outline = Color(0xFF85858F),
                outlineVariant = Color(0xFF3C3C44),
                onBackground = Color(0xFFF4F4F7),
                onSurface = Color(0xFFF4F4F7),
                onSurfaceVariant = Color(0xFFB9BBC4),
            )
        else
            lightColorScheme(
                primary = Color(0xFF0060C5),
                primaryContainer = Color(0xFFD9E9FF),
                secondaryContainer = Color(0xFFD9E9FF),
                onSecondaryContainer = Color(0xFF00325A),
                background = Color(0xFFF9FAFC),
                surface = Color(0xFFF4F4F7),
                surfaceContainer = Color(0xFFECEEF3),
                surfaceContainerLow = Color(0xFFF4F4F7),
                surfaceContainerHigh = Color(0xFFE6E8ED),
                surfaceContainerHighest = Color(0xFFDEE1E7),
                outline = Color(0xFF777C85),
                outlineVariant = Color(0xFFD9DCE3),
                onBackground = Color(0xFF191B21),
                onSurface = Color(0xFF191B21),
                onSurfaceVariant = Color(0xFF555D69),
            )
    MaterialTheme(
        colorScheme = colors,
        typography = remember(messageAppearance.font) { messageTypography(messageAppearance.font) },
    ) {
        val haze = remember { HazeState() }
        CompositionLocalProvider(
            LocalHaze provides haze,
            LocalClientPreferences provides appearance,
            LocalUiDimensions provides UiDimensions(appearance.uiDensity),
            LocalMessageAppearance provides messageAppearance,
            LocalGlass provides glass,
            LocalContentColor provides colors.onSurface,
        ) {
            ExternalLinkHost(model::offerRecovery) {
                val recoveryLink by model.recoveryLink.collectAsStateWithLifecycle()
                if (recoveryLink != null)
                    AccountRecovery(
                        recoveryLink!!,
                        state.active?.url?.takeUnless { it.startsWith("demo:") }.orEmpty(),
                        model::consumeRecovery,
                    )
                var adding by rememberSaveable { mutableStateOf(false) }
                var tab by rememberSaveable { mutableStateOf("Chats") }
                var settingsOrigin by rememberSaveable { mutableStateOf("Chats") }
                var inConversation by rememberSaveable { mutableStateOf(false) }
                val server = state.active
                val conversationStates = rememberSaveableStateHolder()
                var conversationScope by rememberSaveable { mutableStateOf("") }
                val showingConversation =
                    inConversation && conversationScope == "${server?.url}/${server?.me}"
                var conversationId by rememberSaveable { mutableStateOf("") }
                fun enterConversation(target: ServerState) {
                    conversationScope = "${target.url}/${target.me}"
                    conversationId =
                        model.state.value.servers
                            .firstOrNull {
                                it.url == target.url && it.me == target.me
                            }
                            ?.selectedChannel
                            .orEmpty()
                    inConversation = true
                }
                // Arm removal detection only after this composition has observed
                // the route. Empty bootstrap snapshots must not close a restored chat.
                var conversationWasPresent by
                    remember(server?.url, server?.me, conversationId) {
                        mutableStateOf(false)
                    }
                LaunchedEffect(showingConversation, server?.channels, server?.dms, conversationId) {
                    if (showingConversation && conversationId.isNotBlank()) {
                        val present =
                            (server?.channels.orEmpty() + server?.dms.orEmpty()).any {
                                it.id == conversationId
                            }
                        if (present) conversationWasPresent = true
                        else if (conversationWasPresent) inConversation = false
                    }
                }
                var profileId by remember { mutableStateOf<String?>(null) }
                var profileServerUrl by remember { mutableStateOf("") }
                var profileOwner by remember { mutableStateOf("") }
                val openScopedProfile: (ServerState, String) -> Unit = { target, id ->
                    profileServerUrl = target.url
                    profileOwner = target.me
                    profileId = id
                }
                val profileServer =
                    state.servers.firstOrNull {
                        it.url == profileServerUrl && it.me == profileOwner && it.screen == "chat"
                    }
                var externalProfile by
                    remember(state.activeUrl) {
                        mutableStateOf<io.github.chriexpe.papo.core.ExternalAuthorView?>(null)
                    }
                val openProfile: (String) -> Unit = { id ->
                    externalProfile = null
                    server?.let { openScopedProfile(it, id) }
                }
                val leaveConversation: () -> Unit = { inConversation = false }
                SideEffect {
                    model.visibleConversation(
                        if (showingConversation && profileId == null && externalProfile == null)
                            server?.url.orEmpty()
                        else ""
                    )
                }
                val notificationTarget by model.notificationTarget.collectAsStateWithLifecycle()
                LaunchedEffect(notificationTarget) {
                    val target = notificationTarget ?: return@LaunchedEffect
                    val matches =
                        kotlinx.coroutines
                            .withTimeoutOrNull(20_000) {
                                model.state
                                    .map { current ->
                                        current.servers.filter { candidate ->
                                            candidate.screen == "chat" &&
                                                candidate.me.isNotBlank() &&
                                                (target.server.isBlank() ||
                                                    target.server == candidate.url) &&
                                                (target.owner.isBlank() ||
                                                    target.owner == candidate.me) &&
                                                (candidate.channels + candidate.dms).any {
                                                    it.id == target.channel
                                                }
                                        }
                                    }
                                    .first { it.isNotEmpty() }
                            }
                            .orEmpty()
                    if (matches.size == 1) {
                        val destination = matches.single()
                        model.consumeNotification(target)
                        model.navigate(
                            destination,
                            Action.SelectChannel(target.channel),
                            target.message,
                        ) {
                            adding = false
                            profileId = null
                            externalProfile = null
                            tab = "Chats"
                            enterConversation(destination)
                        }
                    } else
                        model.consumeNotification(
                            target,
                            ui(
                                R.string
                                    .this_notification_no_longer_matches_a_signed_in_conversation
                            ),
                        )
                }

                CompositionLocalProvider(
                    LocalProfileContext provides server?.let { ProfileContext(it, model) },
                    LocalOpenProfile provides openProfile,
                    LocalOpenExternalProfile provides
                        { author ->
                            profileId = null
                            externalProfile = author
                        },
                    LocalMediaOverlayBlocking provides
                        (profileId != null || externalProfile != null),
                ) {
                    // Child dialogs, sheets and the IME consume Back first. Keep the
                    // activity alive until navigation reaches the conversation list.
                    BackHandler(
                        showingConversation ||
                            tab != "Chats" ||
                            profileId != null ||
                            externalProfile != null ||
                            adding
                    ) {
                        when {
                            adding -> adding = false
                            profileId != null -> profileId = null
                            externalProfile != null -> externalProfile = null
                            showingConversation -> leaveConversation()
                            tab == "GlobalDirect" || tab == "GlobalNotifications" -> tab = "Servers"
                            tab == "Settings" -> tab = settingsOrigin
                            else -> tab = "Chats"
                        }
                    }
                    Box(Modifier.fillMaxSize().background(colors.background)) {
                        if (state.servers.isEmpty()) {
                            Welcome(
                                state.opening,
                                state.error,
                                model::clearError,
                                model::exploreDemo,
                                { model.offerRecovery("") },
                            ) {
                                model.addServer(it)
                            }
                        } else if (server == null) {
                            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                CircularProgressIndicator()
                            }
                        } else if (
                            server.screen != "chat" &&
                                tab !in listOf("Servers", "GlobalDirect", "GlobalNotifications")
                        ) {
                            Authentication(
                                server,
                                state.error,
                                onBack = { tab = "Servers" },
                                onSwitch = { adding = true },
                                onRecovery = { model.offerRecovery("") },
                                command = { model.command(it) },
                            )
                        } else if (showingConversation && server.conversation != null) {
                            ConversationScreen(server, model, onBack = leaveConversation)
                        } else {
                            Box(Modifier.fillMaxSize()) {
                                when (tab) {
                                    "Chats",
                                    "Direct" ->
                                        conversationStates.SaveableStateProvider(
                                            "${server.url}/${server.me}/$tab"
                                        ) {
                                            ConversationList(
                                                server,
                                                model,
                                                appError = state.error,
                                                direct = tab == "Direct",
                                                onSettings = {
                                                    settingsOrigin = tab
                                                    tab = "Settings"
                                                },
                                                onOpen = { id ->
                                                    model.navigate(
                                                        server,
                                                        Action.SelectChannel(id),
                                                    ) {
                                                        enterConversation(server)
                                                    }
                                                },
                                                onProfile = openProfile,
                                                onMessage = { channelId, messageId ->
                                                    model.navigate(
                                                        server,
                                                        Action.SelectChannel(channelId),
                                                        messageId,
                                                    ) {
                                                        enterConversation(server)
                                                    }
                                                },
                                            )
                                        }
                                    "Servers" ->
                                        ServerList(
                                            state,
                                            model,
                                            onAdd = { adding = true },
                                            onSelect = {
                                                inConversation = false
                                                model.activate(it)
                                                tab = "Chats"
                                            },
                                            onDirect = { tab = "GlobalDirect" },
                                            onNotifications = { tab = "GlobalNotifications" },
                                        )
                                    "GlobalDirect" ->
                                        conversationStates.SaveableStateProvider("global/direct") {
                                            GlobalDirectMessages(
                                                state,
                                                model,
                                                onBack = { tab = "Servers" },
                                                onProfile = openScopedProfile,
                                                onOpen = { target, channel ->
                                                    model.navigate(
                                                        target,
                                                        Action.SelectChannel(channel.id),
                                                    ) {
                                                        enterConversation(target)
                                                    }
                                                },
                                            )
                                        }
                                    "GlobalNotifications" ->
                                        conversationStates.SaveableStateProvider(
                                            "global/notifications"
                                        ) {
                                            GlobalNotifications(
                                                state,
                                                model,
                                                onBack = { tab = "Servers" },
                                                onOpen = { target, item ->
                                                    model.navigate(
                                                        target,
                                                        Action.SelectChannel(item.channelId),
                                                        item.messageId.takeIf { it.isNotBlank() },
                                                    ) {
                                                        enterConversation(target)
                                                    }
                                                },
                                            )
                                        }
                                    else ->
                                        ClientSettings(
                                            server,
                                            model,
                                            onBack = { tab = settingsOrigin },
                                            onLogout = {
                                                if (server.isDemo) model.leaveDemo()
                                                else model.command(Action.Logout)
                                            },
                                        )
                                }
                                if (tab != "Settings")
                                    FloatingNavigation(
                                        if (tab == "GlobalDirect" || tab == "GlobalNotifications")
                                            "Servers"
                                        else tab,
                                        onSelect = { tab = it },
                                        Modifier.align(Alignment.BottomCenter)
                                            .navigationBarsPadding()
                                            .padding(16.dp),
                                    )
                            }
                        }
                        val error =
                            if (state.servers.isEmpty()) ""
                            else state.error.ifBlank { server?.error.orEmpty() }
                        if (error.isNotBlank() && server?.screen == "chat") {
                            FeedbackPill(
                                error,
                                model::clearError,
                                Modifier.align(Alignment.TopCenter)
                                    .statusBarsPadding()
                                    .padding(12.dp),
                            )
                        }
                    }
                    if (server?.screen == "chat")
                        externalProfile?.let { author ->
                            ExternalProfileSheet(server, author) { externalProfile = null }
                        }
                    profileServer?.let { target ->
                        profileId?.let { id ->
                            CompositionLocalProvider(
                                LocalProfileContext provides ProfileContext(target, model)
                            ) {
                                UserProfileSheet(
                                    target,
                                    model,
                                    id,
                                    onDismiss = { profileId = null },
                                    onMessage = { user ->
                                        profileId = null
                                        model.navigate(target, Action.OpenDm(user)) {
                                            enterConversation(target)
                                        }
                                    },
                                )
                            }
                        }
                    }
                }
                if (adding)
                    ModalBottomSheet(onDismissRequest = { adding = false }) {
                        AddServerForm(
                            state.opening,
                            state.error,
                            modifier = Modifier.padding(horizontal = 24.dp).navigationBarsPadding(),
                        ) {
                            model.addServer(it) { adding = false }
                        }
                        Spacer(Modifier.height(24.dp))
                    }
            }
        }
    }
}
