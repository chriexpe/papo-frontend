@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import io.github.chriexpe.papo.core.AccountChange
import io.github.chriexpe.papo.core.SessionView
import java.time.OffsetDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun SessionSettings(
    server: ServerState,
    model: PapoViewModel,
    feedback: (String) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var sessions by remember { mutableStateOf<List<SessionView>?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var ending by remember { mutableStateOf<SessionView?>(null) }
    suspend fun load() {
        busy = true
        error = ""
        try {
            sessions = model.accountSessions(server)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: ui(R.string.couldn_t_load_sessions)
        } finally {
            busy = false
        }
    }
    LaunchedEffect(server.url, server.me) { load() }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Text(
                ui(R.string.signed_in_to_1_s, server.name),
                Modifier.padding(8.dp),
                style = MaterialTheme.typography.titleMedium,
            )
        }
        item {
            Text(
                ui(
                    R.string
                        .the_server_provides_session_ids_and_dates_but_doesn_t_identify_devices_or_w
                ),
                Modifier.padding(8.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (busy) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
        if (error.isNotBlank()) item { SettingsError(error) { scope.launch { load() } } }
        items(sessions.orEmpty(), key = { it.id }) { session ->
            SettingsCard {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        ui(R.string.session_short, session.id.takeLast(8)),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    if (session.createdAt.isNotBlank())
                        Text(
                            ui(R.string.created_at, settingsDate(session.createdAt)),
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    if (session.expiresAt.isNotBlank())
                        Text(
                            ui(R.string.expires_at, settingsDate(session.expiresAt)),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    TextButton(enabled = !busy, onClick = { ending = session }) {
                        Text(ui(R.string.end_session))
                    }
                }
            }
        }
        if (sessions?.isEmpty() == true && !busy)
            item {
                Text(ui(R.string.the_server_returned_no_active_sessions), Modifier.padding(16.dp))
            }
        item {
            OutlinedButton(enabled = !busy, onClick = { scope.launch { load() } }) {
                Icon(Icons.Outlined.Refresh, null)
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.refresh_sessions))
            }
        }
    }
    ending?.let { session ->
        AlertDialog(
            onDismissRequest = { ending = null },
            title = { Text(ui(R.string.end_this_session)) },
            text = {
                Text(
                    ui(
                        R.string
                            .session_1_s_will_be_signed_out_if_it_belongs_to_this_app_you_ll_need_to_sig,
                        session.id.takeLast(8),
                    )
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        ending = null
                        busy = true
                        scope.launch {
                            try {
                                model.accountChange(server, AccountChange.RevokeSession(session.id))
                                sessions = sessions.orEmpty().filterNot { it.id == session.id }
                                feedback(ui(R.string.session_ended))
                                load()
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                error = e.message ?: ui(R.string.couldn_t_end_session)
                            } finally {
                                busy = false
                            }
                        }
                    }
                ) {
                    Text(ui(R.string.end_session))
                }
            },
            dismissButton = {
                TextButton(onClick = { ending = null }) { Text(ui(R.string.cancel)) }
            },
        )
    }
}

private fun settingsDate(raw: String) = runCatching {
    OffsetDateTime.parse(raw)
        .atZoneSameInstant(java.time.ZoneId.systemDefault())
        .format(
            DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
                .withLocale(AppLanguage.locale)
        )
}
    .getOrDefault(raw)
