@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.HideSource
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.BlockedPerson
import io.github.chriexpe.papo.core.SocialException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun DirectMessageOptions(
    server: ServerState,
    model: PapoViewModel,
    channel: Conversation,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 24.dp).navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(channel.name, style = MaterialTheme.typography.titleLarge)
            Text(
                ui(R.string.hide_dm_explanation),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (error.isNotBlank()) SettingsError(error)
            TextButton(
                enabled = !busy,
                modifier = Modifier.testTag("hide-direct-message"),
                onClick = {
                    busy = true
                    error = ""
                    scope.launch {
                        try {
                            model.hideDirectMessage(server, channel.id)
                            onDismiss()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            error = socialFailure(e)
                        } finally {
                            busy = false
                        }
                    }
                },
            ) {
                Icon(Icons.Outlined.HideSource, null)
                Spacer(Modifier.width(8.dp))
                Text(ui(R.string.hide_conversation))
            }
            Spacer(Modifier.height(16.dp))
        }
    }
}

/** Protocol failures remain semantic across UniFFI; only presentation is translated. */
internal fun socialFailure(error: Exception): String =
    when (error) {
        is SocialException.AccountChanged -> ui(R.string.account_changed)
        is SocialException.InvalidTarget -> ui(R.string.social_target_unavailable)
        is SocialException.Blocked -> ui(R.string.dm_blocked)
        is SocialException.SignedOut -> ui(R.string.social_sign_in_again)
        is SocialException.Unavailable -> ui(R.string.social_network_failure)
        else -> ui(R.string.social_request_failed)
    }

@Composable
internal fun BlockPersonDialog(
    server: ServerState,
    model: PapoViewModel,
    id: String,
    name: String,
    blocked: Boolean,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = {
            Text(
                ui(
                    if (blocked) R.string.unblock_person_title else R.string.block_person_title,
                    name,
                )
            )
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    ui(
                        if (blocked) R.string.unblock_person_explanation
                        else R.string.block_person_explanation
                    )
                )
                if (error.isNotBlank()) SettingsError(error)
            }
        },
        confirmButton = {
            TextButton(
                enabled = !busy,
                onClick = {
                    busy = true
                    error = ""
                    scope.launch {
                        try {
                            model.setPersonBlocked(server, id, !blocked)
                            onDismiss()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            error = socialFailure(e)
                        } finally {
                            busy = false
                        }
                    }
                },
                modifier = Modifier.testTag("confirm-person-block"),
            ) {
                Text(ui(if (blocked) R.string.unblock_person else R.string.block_person))
            }
        },
        dismissButton = {
            TextButton(enabled = !busy, onClick = onDismiss) { Text(ui(R.string.cancel)) }
        },
    )
}

@Composable
internal fun BlockedPeopleSettings(server: ServerState, model: PapoViewModel) {
    // Ownership keys discard results and local busy/error state on account changes.
    key(server.url, server.me) {
        var people by remember { mutableStateOf<List<BlockedPerson>>(emptyList()) }
        var loading by remember { mutableStateOf(true) }
        var error by remember { mutableStateOf("") }
        var retry by remember { mutableIntStateOf(0) }
        var pending by remember { mutableStateOf<Set<String>>(emptySet()) }
        val scope = rememberCoroutineScope()
        LaunchedEffect(retry) {
            loading = true
            error = ""
            try {
                people = model.blockedPeople(server)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = socialFailure(e)
            } finally {
                loading = false
            }
        }
        LazyColumn(
            Modifier.fillMaxSize().testTag("blocked-people"),
            contentPadding = PaddingValues(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Text(
                    ui(R.string.blocked_people_on_server, server.name),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
            item {
                Text(
                    ui(R.string.blocked_people_summary),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (loading) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
            if (error.isNotBlank()) item { SettingsError(error) { retry++ } }
            if (!loading && error.isBlank() && people.isEmpty())
                item { Text(ui(R.string.no_blocked_people)) }
            items(people, key = { it.id }) { person ->
                SettingsCard {
                    ListItem(
                        headlineContent = {
                            Text(person.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        },
                        supportingContent = {
                            Text(
                                "@${person.username}",
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        },
                        leadingContent = { UserAvatar(person.id, person.name) },
                    )
                    TextButton(
                        enabled = person.id !in pending,
                        modifier = Modifier.testTag("unblock-${person.id}"),
                        onClick = {
                            pending = pending + person.id
                            error = ""
                            scope.launch {
                                try {
                                    model.setPersonBlocked(server, person.id, false)
                                    people = people.filterNot { it.id == person.id }
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (e: Exception) {
                                    error = socialFailure(e)
                                } finally {
                                    pending = pending - person.id
                                }
                            }
                        },
                    ) {
                        Text(ui(R.string.unblock_person))
                    }
                }
            }
        }
    }
}
