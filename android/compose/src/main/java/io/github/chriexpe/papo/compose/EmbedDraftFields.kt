@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.*

@Composable
internal fun EmbedForm(
    d: EmbedDraft,
    files: List<PendingAttachment>,
    onChange: (EmbedDraft) -> Unit,
) {
    EmbedText(d.title, R.string.embed_title, 256) { onChange(d.copy(title = it)) }
    EmbedText(d.description, R.string.embed_description, 4192, multiline = true) {
        onChange(d.copy(description = it))
    }
    EmbedText(d.url, R.string.embed_url, 2048) { onChange(d.copy(url = it)) }
    EmbedText(d.color, R.string.embed_color, 7) { onChange(d.copy(color = it)) }
    EmbedText(d.site, R.string.embed_site, 256) { onChange(d.copy(site = it)) }
    EmbedSection(ui(R.string.embed_author_footer)) {
        EmbedText(d.author, R.string.embed_author, 256) { onChange(d.copy(author = it)) }
        EmbedText(d.authorUrl, R.string.embed_author_url, 2048) { onChange(d.copy(authorUrl = it)) }
        EmbedText(d.authorIcon, R.string.embed_author_icon, 2048) {
            onChange(d.copy(authorIcon = it))
        }
        EmbedText(d.footer, R.string.embed_footer, 2048, multiline = true) {
            onChange(d.copy(footer = it))
        }
    }
    EmbedSection(ui(R.string.embed_media)) {
        Text(ui(R.string.embed_media_note), style = MaterialTheme.typography.bodySmall)
        EmbedImageInput(ui(R.string.embed_image), d.image, files) { onChange(d.copy(image = it)) }
        EmbedImageInput(ui(R.string.embed_thumbnail), d.thumbnail, files) {
            onChange(d.copy(thumbnail = it))
        }
        EmbedText(d.video?.url.orEmpty(), R.string.embed_video, 2048) { url ->
            onChange(
                d.copy(
                    video =
                        url.takeIf { it.isNotBlank() }
                            ?.let {
                                (d.video ?: newEmbedMedia().copy(mime = "video/mp4")).copy(url = it)
                            }
                )
            )
        }
        d.video?.let { video ->
            var menu by remember { mutableStateOf(false) }
            Box {
                OutlinedButton(onClick = { menu = true }) { Text(video.mime) }
                DropdownMenu(menu, onDismissRequest = { menu = false }) {
                    listOf("video/mp4", "video/webm", "video/ogg").forEach { mime ->
                        DropdownMenuItem(
                            text = { Text(mime) },
                            onClick = {
                                menu = false
                                onChange(d.copy(video = video.copy(mime = mime)))
                            },
                        )
                    }
                }
            }
        }
    }
    EmbedSection(ui(R.string.embed_fields, d.fields.size)) {
        d.fields.forEachIndexed { index, field ->
            ElevatedCard {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    fun update(next: EmbedFieldView) =
                        onChange(
                            d.copy(fields = d.fields.toMutableList().also { it[index] = next })
                        )
                    EmbedText(field.name, R.string.embed_field_name, 256) {
                        update(field.copy(name = it))
                    }
                    EmbedText(field.value, R.string.embed_field_value, 1024, multiline = true) {
                        update(field.copy(value = it))
                    }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(ui(R.string.embed_field_inline), Modifier.weight(1f))
                        Switch(field.inline, onCheckedChange = { update(field.copy(inline = it)) })
                        IconButton(
                            onClick = {
                                onChange(
                                    d.copy(fields = d.fields.filterIndexed { i, _ -> i != index })
                                )
                            }
                        ) {
                            Icon(Icons.Outlined.DeleteOutline, ui(R.string.embed_remove_field))
                        }
                    }
                }
            }
        }
        OutlinedButton(
            enabled = d.fields.size < 25,
            onClick = { onChange(d.copy(fields = d.fields + EmbedFieldView("", "", false))) },
        ) {
            Text(ui(R.string.embed_add_field))
        }
    }
}

@Composable
private fun EmbedSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    OutlinedCard(Modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().clickable { expanded = !expanded }.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
            Icon(
                if (expanded) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore,
                if (expanded) ui(R.string.embed_collapse) else ui(R.string.embed_expand),
            )
        }
        if (expanded)
            Column(
                Modifier.padding(horizontal = 16.dp).padding(bottom = 16.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
                content = content,
            )
    }
}

@Composable
private fun EmbedText(
    value: String,
    label: Int,
    limit: Int,
    multiline: Boolean = false,
    onChange: (String) -> Unit,
) {
    OutlinedTextField(
        value,
        onValueChange = onChange,
        label = { Text(ui(label)) },
        modifier = Modifier.fillMaxWidth().testTag("embed-field-$label"),
        singleLine = !multiline,
        minLines = if (multiline) 3 else 1,
        maxLines = if (multiline) 8 else 1,
        isError = value.codePointsCount() > limit,
        supportingText = { if (value.isNotEmpty()) Text("${value.codePointsCount()} / $limit") },
    )
}

@Composable
private fun EmbedImageInput(
    label: String,
    value: EmbedMediaDraft?,
    files: List<PendingAttachment>,
    onChange: (EmbedMediaDraft?) -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    Text(label, style = MaterialTheme.typography.labelLarge)
    if (!value?.attachment.isNullOrEmpty()) {
        InputChip(
            true,
            onClick = { menu = true },
            label = {
                Text(
                    value!!.attachment,
                    maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
            },
            trailingIcon = {
                IconButton(onClick = { onChange(null) }) {
                    Icon(Icons.Outlined.Close, ui(R.string.remove))
                }
            },
        )
    } else
        OutlinedTextField(
            value?.url.orEmpty(),
            onValueChange = { url ->
                onChange(url.takeIf { it.isNotBlank() }?.let { newEmbedMedia().copy(url = it) })
            },
            label = { Text(ui(R.string.embed_https_image)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
    val choices = files.filter {
        it.uploadMime in listOf("image/png", "image/jpeg", "image/webp", "image/gif") &&
            !it.wireName.startsWith("SPOILER_") &&
            files.count { other -> other.wireName == it.wireName } == 1
    }
    if (choices.isNotEmpty())
        Box {
            TextButton(onClick = { menu = true }) { Text(ui(R.string.embed_use_attachment)) }
            DropdownMenu(menu, onDismissRequest = { menu = false }) {
                choices.forEach { file ->
                    DropdownMenuItem(
                        text = { Text(file.name) },
                        onClick = {
                            menu = false
                            onChange(
                                newEmbedMedia()
                                    .copy(attachment = file.wireName, mime = file.uploadMime)
                            )
                        },
                    )
                }
            }
        }
}
