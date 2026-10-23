@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.chriexpe.papo.core.*

internal fun restoredEmbeds(json: String): List<EmbedDraft> = runCatching {
    decodeEmbedDrafts(json)
}
    .getOrDefault(emptyList())

/** Cancel discards the staged list; Apply commits the complete list atomically to its owner. */
@Composable
internal fun EmbedDraftEditor(
    original: List<EmbedDraft>,
    files: List<PendingAttachment>,
    server: ServerState,
    model: PapoViewModel,
    onApply: (List<EmbedDraft>) -> Unit,
    onDismiss: () -> Unit,
    initialIndex: Int = 0,
    saving: Boolean = false,
    saveError: String = "",
    editingMessage: Boolean = false,
) {
    var json by rememberSaveable {
        mutableStateOf(encodeEmbedDrafts(original.ifEmpty { listOf(newEmbedDraft()) }))
    }
    val drafts = remember(json) { restoredEmbeds(json) }
    var selected by rememberSaveable { mutableIntStateOf(initialIndex) }
    var preview by rememberSaveable { mutableStateOf(false) }
    var confirmDiscard by remember { mutableStateOf(false) }
    var contentBounds by remember { mutableStateOf<Rect?>(null) }
    val issues = remember(json, files) { validateEmbedDrafts(drafts, files.map { it.wireName }) }
    val current = selected.coerceIn(0, (drafts.size - 1).coerceAtLeast(0))
    fun replace(next: List<EmbedDraft>) {
        if (!saving) json = encodeEmbedDrafts(next)
    }
    fun close() {
        if (saving) return
        if (drafts == original || (original.isEmpty() && drafts == listOf(newEmbedDraft())))
            onDismiss()
        else confirmDiscard = true
    }
    Dialog(
        onDismissRequest = ::close,
        properties =
            DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Column(
                Modifier.fillMaxSize().safeDrawingPadding().imePadding().testTag("embed-editor")
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(onClick = ::close) {
                        Icon(Icons.Outlined.Close, ui(R.string.cancel))
                    }
                    Text(
                        ui(R.string.embed_editor_title),
                        Modifier.weight(1f),
                        style = MaterialTheme.typography.titleLarge,
                    )
                    TextButton(
                        enabled = !saving && issues.isEmpty(),
                        onClick = { onApply(drafts) },
                    ) {
                        Text(ui(if (editingMessage) R.string.save else R.string.embed_apply))
                    }
                }
                LazyRow(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    itemsIndexed(drafts) { index, _ ->
                        FilterChip(
                            selected = current == index,
                            onClick = { selected = index },
                            label = { Text(ui(R.string.embed_number, index + 1)) },
                        )
                    }
                    item {
                        AssistChip(
                            enabled = drafts.size < 10,
                            onClick = {
                                replace(drafts + newEmbedDraft())
                                selected = drafts.size
                                preview = false
                            },
                            label = { Text(ui(R.string.embed_add)) },
                            leadingIcon = { Icon(Icons.Outlined.Add, null) },
                        )
                    }
                }
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilterChip(
                        !preview,
                        onClick = { preview = false },
                        label = { Text(ui(R.string.embed_edit_tab)) },
                    )
                    FilterChip(
                        preview,
                        onClick = { preview = true },
                        label = { Text(ui(R.string.embed_preview)) },
                    )
                    Spacer(Modifier.weight(1f))
                    if (drafts.isNotEmpty()) {
                        IconButton(
                            enabled = current > 0,
                            onClick = {
                                val next = drafts.toMutableList()
                                java.util.Collections.swap(next, current, current - 1)
                                replace(next)
                                selected = current - 1
                            },
                        ) {
                            Icon(Icons.Outlined.ArrowBack, ui(R.string.embed_move_before))
                        }
                        IconButton(
                            enabled = current < drafts.lastIndex,
                            onClick = {
                                val next = drafts.toMutableList()
                                java.util.Collections.swap(next, current, current + 1)
                                replace(next)
                                selected = current + 1
                            },
                        ) {
                            Icon(Icons.Outlined.ArrowForward, ui(R.string.embed_move_after))
                        }
                    }
                }
                Column(
                    Modifier.weight(1f)
                        .onGloballyPositioned { contentBounds = it.boundsInWindow() }
                        .verticalScroll(rememberScrollState())
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (saveError.isNotEmpty())
                        Text(saveError, color = MaterialTheme.colorScheme.error)
                    issues.forEach {
                        Text(
                            embedIssueText(it),
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    if (drafts.isEmpty()) Text(ui(R.string.embed_no_cards))
                    else if (preview) {
                        Text(
                            ui(R.string.embed_preview_note),
                            style = MaterialTheme.typography.bodySmall,
                        )
                        CompositionLocalProvider(LocalMediaViewport provides contentBounds) {
                            EmbedAuthoringPreview(drafts[current], files, server, model)
                        }
                    } else
                        key(current) {
                            EmbedForm(
                                drafts[current],
                                files,
                                onChange = { next ->
                                    replace(drafts.toMutableList().also { it[current] = next })
                                },
                            )
                            OutlinedButton(
                                onClick = {
                                    replace(drafts.filterIndexed { index, _ -> index != current })
                                    selected = (current - 1).coerceAtLeast(0)
                                }
                            ) {
                                Icon(Icons.Outlined.DeleteOutline, null)
                                Spacer(Modifier.width(8.dp))
                                Text(ui(R.string.embed_remove))
                            }
                        }
                }
            }
        }
        if (confirmDiscard)
            AlertDialog(
                onDismissRequest = { confirmDiscard = false },
                title = { Text(ui(R.string.embed_discard_title)) },
                text = { Text(ui(R.string.embed_discard_description)) },
                confirmButton = { TextButton(onClick = onDismiss) { Text(ui(R.string.discard)) } },
                dismissButton = {
                    TextButton(onClick = { confirmDiscard = false }) { Text(ui(R.string.cancel)) }
                },
            )
        // Keep the staged cards mounted on failure, and prevent edits/back navigation while
        // the server mutation is pending. The owner's unsent chat draft is never involved.
        if (saving)
            AlertDialog(
                onDismissRequest = {},
                properties =
                    DialogProperties(dismissOnBackPress = false, dismissOnClickOutside = false),
                confirmButton = {},
                text = {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        CircularProgressIndicator(Modifier.size(24.dp))
                        Text(ui(R.string.saving))
                    }
                },
            )
    }
}

private fun embedIssueText(issue: EmbedIssue): String {
    val field =
        ui(
            when (issue.field) {
                "title" -> R.string.embed_title
                "description" -> R.string.embed_description
                "site_name" -> R.string.embed_site
                "author" -> R.string.embed_author
                "footer" -> R.string.embed_footer
                "color" -> R.string.embed_color
                "url" -> R.string.embed_url
                "author_url" -> R.string.embed_author_url
                "author_icon" -> R.string.embed_author_icon
                "image" -> R.string.embed_image
                "thumbnail" -> R.string.embed_thumbnail
                "video" -> R.string.embed_video
                "field_name" -> R.string.embed_field_name
                "field_value" -> R.string.embed_field_value
                "fields" -> R.string.embed_fields_label
                "total" -> R.string.embed_total
                else -> R.string.embed_editor_title
            }
        )
    val reason =
        when (issue.code) {
            "limit" -> ui(R.string.embed_limit, issue.limit.toInt())
            "color" -> ui(R.string.embed_invalid_color)
            "attachment" -> ui(R.string.embed_invalid_attachment)
            "mime" -> ui(R.string.embed_invalid_mime)
            "empty" -> ui(R.string.embed_empty_card)
            else -> ui(R.string.embed_invalid_url)
        }
    return ui(R.string.embed_issue, issue.index.toInt() + 1, field, reason)
}
