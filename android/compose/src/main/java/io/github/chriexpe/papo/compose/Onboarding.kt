@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.Action

private val Blue = Color(0xFF0A84FF)

@Composable
internal fun Welcome(
    opening: Boolean,
    error: String,
    clearError: () -> Unit,
    onDemo: () -> Unit,
    onRecovery: () -> Unit,
    onConnect: (String) -> Unit,
) {
    Box(Modifier.fillMaxSize().imePadding()) {
        Box(Modifier.fillMaxSize().hazeSource(LocalHaze.current)) {
            Canvas(Modifier.fillMaxSize()) {
                drawCircle(
                    Brush.radialGradient(
                        listOf(Blue.copy(alpha = 0.14f), Color.Transparent),
                        center = Offset(size.width * .8f, size.height * .18f),
                        radius = size.width * .8f,
                    ),
                    radius = size.width,
                    center = Offset(size.width * .8f, size.height * .18f),
                )
            }
        }
        Column(
            Modifier.fillMaxSize()
                .statusBarsPadding()
                .navigationBarsPadding()
                .padding(horizontal = 28.dp)
                .verticalScrollCompat(),
            verticalArrangement = Arrangement.Center,
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                BrandMark()
                Text(
                    "papo",
                    fontSize = 28.sp,
                    fontWeight = FontWeight.Bold,
                    letterSpacing = (-1).sp,
                )
            }
            var choosingLanguage by remember { mutableStateOf(false) }
            TextButton(onClick = { choosingLanguage = true }) {
                Icon(Icons.Outlined.Language, null)
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.language))
            }
            if (choosingLanguage)
                ModalBottomSheet(onDismissRequest = { choosingLanguage = false }) {
                    LanguageSettings()
                    Spacer(Modifier.navigationBarsPadding().height(24.dp))
                }
            Spacer(Modifier.height(32.dp))
            Text(
                ui(R.string.good_conversations_start_here),
                fontSize = 38.sp,
                lineHeight = 43.sp,
                letterSpacing = (-1.5).sp,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(16.dp))
            Text(
                ui(
                    R.string
                        .a_place_for_your_people_connect_to_a_papo_server_to_make_yourself_at_home
                ),
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(40.dp))
            AddServerForm(opening, error, showHeading = false, onConnect = onConnect)
            TextButton(onClick = onRecovery, Modifier.align(Alignment.CenterHorizontally)) {
                Text(ui(R.string.use_a_recovery_link))
            }
            TextButton(onClick = onDemo, Modifier.align(Alignment.CenterHorizontally)) {
                Text(ui(R.string.explore_demo))
            }
            if (error.isNotBlank())
                TextButton(onClick = clearError) { Text(ui(R.string.dismiss_error)) }
            Spacer(Modifier.height(40.dp))
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Outlined.Lock,
                    null,
                    Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    ui(R.string.your_server_your_community),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun BrandMark() {
    Box(
        Modifier.size(46.dp).clip(RoundedCornerShape(15.dp)).background(Blue),
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Outlined.Forum, null, Modifier.size(26.dp), Color.White)
    }
}

@Composable
internal fun AddServerForm(
    opening: Boolean,
    error: String,
    modifier: Modifier = Modifier,
    showHeading: Boolean = true,
    onConnect: (String) -> Unit,
) {
    var address by rememberSaveable { mutableStateOf("") }
    val keyboard = LocalSoftwareKeyboardController.current
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (showHeading) {
            Text(
                ui(R.string.add_a_server),
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                ui(R.string.enter_the_address_shared_by_your_community),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        OutlinedTextField(
            address,
            { address = it },
            label = { Text(ui(R.string.server_address)) },
            supportingText = { Text(ui(R.string.use_the_address_shared_by_your_community)) },
            leadingIcon = { Icon(Icons.Outlined.Dns, null) },
            singleLine = true,
            keyboardOptions =
                KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { keyboard?.hide() }),
            shape = RoundedCornerShape(18.dp),
            modifier = Modifier.fillMaxWidth(),
            enabled = !opening,
        )
        if (error.isNotBlank())
            Text(
                error,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
            )
        Button(
            onClick = { onConnect(address.trim()) },
            enabled = address.isNotBlank() && !opening,
            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
            contentPadding = PaddingValues(16.dp),
        ) {
            if (opening) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            else {
                Icon(Icons.Outlined.Add, null, Modifier.size(20.dp))
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.add_server))
            }
        }
    }
}

@Composable
internal fun Authentication(
    server: ServerState,
    appError: String,
    onBack: () -> Unit,
    onSwitch: () -> Unit,
    onRecovery: () -> Unit,
    command: (Action) -> Unit,
) {
    var username by rememberSaveable(server.url) { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var register by rememberSaveable(server.url) { mutableStateOf(false) }
    Column(
        Modifier.fillMaxSize()
            .imePadding()
            .statusBarsPadding()
            .navigationBarsPadding()
            .padding(24.dp)
            .verticalScrollCompat(),
        verticalArrangement = Arrangement.Center,
    ) {
        BrandMark()
        Spacer(Modifier.height(32.dp))
        Text(
            if (server.locked) ui(R.string.a_little_privacy)
            else if (server.screen == "needs_server") ui(R.string.make_it_yours)
            else ui(R.string.welcome_to_1_s, server.name),
            style = MaterialTheme.typography.headlineLarge,
            fontWeight = FontWeight.SemiBold,
        )
        Text(
            server.url,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(vertical = 12.dp),
        )
        if (server.screen == "starting") {
            LinearProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 20.dp))
            Text(
                ui(R.string.connecting_to_your_server),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            if (!server.locked)
                OutlinedTextField(
                    username,
                    { username = it },
                    label = {
                        Text(
                            if (server.screen == "needs_server") ui(R.string.server_name)
                            else "Username"
                        )
                    },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !server.busy,
                    shape = RoundedCornerShape(16.dp),
                )
            if (server.screen != "needs_server")
                OutlinedTextField(
                    password,
                    { password = it },
                    label = {
                        Text(if (server.locked) ui(R.string.server_password) else "Password")
                    },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                    enabled = !server.busy,
                    shape = RoundedCornerShape(16.dp),
                )
            val error = appError.ifBlank { server.error }
            if (error.isNotBlank())
                Text(
                    error,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(vertical = 12.dp),
                )
            Button(
                onClick = {
                    when {
                        server.locked -> command(Action.Unlock(password))
                        server.screen == "needs_server" -> command(Action.CreateServer(username))
                        else ->
                            command(
                                if (register) Action.Register(username, password)
                                else Action.Login(username, password)
                            )
                    }
                    password = ""
                },
                enabled =
                    !server.busy &&
                        (if (server.locked) password.isNotBlank()
                        else if (server.screen == "needs_server") username.isNotBlank()
                        else username.isNotBlank() && password.isNotBlank()),
                modifier = Modifier.fillMaxWidth().padding(top = 24.dp).heightIn(min = 52.dp),
            ) {
                if (server.busy) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                else
                    Text(
                        if (server.locked) ui(R.string.unlock_server)
                        else if (server.screen == "needs_server") ui(R.string.create_server)
                        else if (register) ui(R.string.create_account) else ui(R.string.sign_in)
                    )
            }
            if (!server.locked && server.screen == "auth")
                TextButton(
                    onClick = { register = !register },
                    Modifier.align(Alignment.CenterHorizontally),
                ) {
                    Text(
                        if (register) ui(R.string.already_have_an_account_sign_in)
                        else ui(R.string.new_here_create_an_account)
                    )
                }
        }
        if (server.screen == "auth" && !server.locked)
            TextButton(onClick = onRecovery, Modifier.align(Alignment.CenterHorizontally)) {
                Text(ui(R.string.use_a_recovery_link))
            }
        TextButton(
            onClick = onSwitch,
            Modifier.align(Alignment.CenterHorizontally).padding(top = 20.dp),
        ) {
            Text(ui(R.string.connect_to_another_server))
        }
        TextButton(onClick = onBack, Modifier.align(Alignment.CenterHorizontally)) {
            Text(ui(R.string.your_servers))
        }
    }
}

@Composable
private fun Modifier.verticalScrollCompat(): Modifier =
    this.then(Modifier.verticalScroll(androidx.compose.foundation.rememberScrollState()))
