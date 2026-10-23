@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import io.github.chriexpe.papo.core.*
import java.io.File

/** Preview is explicit, so typing a remote URL never starts a media request. */
@Composable
internal fun EmbedAuthoringPreview(
    draft: EmbedDraft,
    files: List<PendingAttachment>,
    server: ServerState,
    model: PapoViewModel,
) {
    val e = remember(draft) { embedDraftPreview(draft) }
    var active by remember(draft, server.url, server.me) { mutableStateOf<String?>(null) }
    var viewed by remember(draft, server.url, server.me) { mutableStateOf<ViewerMedia?>(null) }
    CompositionLocalProvider(
        LocalInlinePlayer provides active,
        LocalViewedMedia provides viewed,
        LocalMediaViewer provides { viewed = it },
    ) {
        EmbedCardLayout(
            e,
            authorImage = {
                if (draft.authorIcon.isNotBlank())
                    Box(Modifier.size(24.dp)) {
                        MediaImage(
                            server,
                            model,
                            ui(R.string.embed_author_icon),
                            publicUrl = draft.authorIcon,
                            viewer = false,
                        )
                    }
            },
            media = {
                if (draft.video != null)
                    ReservedMedia(0, 0) {
                        if (active == null)
                            (draft.thumbnail ?: draft.image)?.let { image ->
                                EmbedDraftImage(image, draft.title, files, server, model)
                            }
                        VideoPreview(
                            server,
                            model,
                            ViewerMedia(
                                "${server.url}/${server.me}/draft-video/${draft.video!!.url}",
                                draft.title,
                                "video",
                                draft.video!!.mime,
                                0,
                                0,
                                url = draft.video!!.url,
                            ),
                            onPlayer = { active = it },
                            onActions = {},
                        )
                    }
                listOfNotNull((draft.thumbnail ?: draft.image).takeIf { draft.video == null })
                    .forEach { image ->
                        ReservedMedia(0, 0, fallback = 4f / 3f) {
                            EmbedDraftImage(image, draft.title, files, server, model)
                        }
                    }
            },
        )
        viewed?.let { media -> MediaViewer(server, model, media) { viewed = null } }
    }
}

@Composable
private fun BoxScope.EmbedDraftImage(
    image: EmbedMediaDraft,
    title: String,
    files: List<PendingAttachment>,
    server: ServerState,
    model: PapoViewModel,
) {
    val context = LocalContext.current
    if (image.attachment.isNotEmpty()) {
        val file = files.singleOrNull { it.wireName == image.attachment }
        if (file != null)
            ManagedAsyncImage(
                File(file.sanitized?.path ?: file.path),
                title,
                MediaImages.loader(context),
                Modifier.fillMaxSize(),
            )
    } else if (image.url.isNotBlank()) MediaImage(server, model, title, publicUrl = image.url)
}
