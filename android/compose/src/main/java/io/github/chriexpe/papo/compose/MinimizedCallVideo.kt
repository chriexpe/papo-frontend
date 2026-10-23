@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Fullscreen
import androidx.compose.material.icons.outlined.GridView
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

private data class CallVideoPreview(val person: CallParticipant, val screen: Boolean = false) {
    val id
        get() = "${person.id}/$screen"
}

private fun previewVideos(
    session: CallPreviewSession,
    people: List<CallParticipant>,
): List<CallVideoPreview> {
    val screen =
        if (session.layout == CallPreviewLayout.Screen) {
            people
                .firstOrNull { !it.mine }
                ?.let { listOf(CallVideoPreview(it, screen = true)) }
                .orEmpty()
        } else emptyList()
    return screen + people.filter { it.camera }.map { CallVideoPreview(it) }
}

/**
 * Prefer a speaking participant when the limit excludes others; retained tiles keep their order.
 */
private fun visibleVideos(videos: List<CallVideoPreview>, limit: Int): List<CallVideoPreview> {
    val selected =
        videos.sortedByDescending { it.person.speaking }.take(limit).map { it.id }.toSet()
    return videos.filter { it.id in selected }
}

@Composable
internal fun MinimizedCallVideos(
    calls: VoiceCallContext,
    session: CallPreviewSession,
    people: List<CallParticipant>,
) {
    val available = previewVideos(session, people)
    if (available.isEmpty()) return
    val visible = visibleVideos(available, session.videoLimit)
    // Keep the last frames in composition through the closing animation; removing them as soon
    // as the count becomes zero would collapse the surface before the reverse morph can run.
    var lastVisible by remember(session.sessionId) { mutableStateOf(emptyList<CallVideoPreview>()) }
    val shown = visible.ifEmpty { lastVisible }
    SideEffect { if (visible.isNotEmpty()) lastVisible = visible }
    val motion = callMotionEnabled()
    // A newly appearing video section also enters from the compact pill, rather than mounting
    // already expanded when the first camera turns on.
    val visibility = remember(session.sessionId) { MutableTransitionState(false) }
    LaunchedEffect(visible.isNotEmpty()) { visibility.targetState = visible.isNotEmpty() }
    Column(Modifier.fillMaxWidth().padding(bottom = 8.dp).testTag("call-minimized-video")) {
        Row(
            Modifier.fillMaxWidth().padding(start = 12.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                if (visible.isEmpty()) ui(R.string.call_videos_hidden)
                else uiQuantity(R.plurals.call_video_count, visible.size),
                Modifier.weight(1f),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            IconButton(
                onClick = { calls.controller.openVideoLimit(session) },
                modifier =
                    Modifier.size(48.dp).testTag("call-video-count").semantics {
                        contentDescription = ui(R.string.call_choose_video_count)
                    },
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(3.dp),
                ) {
                    Icon(Icons.Outlined.GridView, null, Modifier.size(18.dp))
                    Text(session.videoLimit.toString(), style = MaterialTheme.typography.labelSmall)
                }
            }
            CallIconButton(
                Icons.Outlined.Fullscreen,
                ui(R.string.call_expand_video),
                tag = "call-video-expand",
                onClick = calls.controller::reopen,
            )
        }
        AnimatedVisibility(
            visibleState = visibility,
            enter =
                if (motion)
                    expandVertically(
                        spring(dampingRatio = .9f, stiffness = 500f),
                        expandFrom = Alignment.Top,
                    ) + fadeIn(tween(120))
                else EnterTransition.None,
            exit =
                if (motion)
                    shrinkVertically(
                        spring(dampingRatio = .9f, stiffness = 500f),
                        shrinkTowards = Alignment.Top,
                    ) + fadeOut(tween(90))
                else ExitTransition.None,
        ) {
            if (shown.isNotEmpty()) MinimizedVideoGrid(shown, calls.controller::reopen)
        }
    }
}

/** Keep the chosen number visible without letting a four-tile grid consume the chat viewport. */
@Composable
private fun MinimizedVideoGrid(videos: List<CallVideoPreview>, onExpand: () -> Unit) {
    val keyboardHeight = WindowInsets.ime.asPaddingValues().calculateBottomPadding()
    val viewportHeight = LocalConfiguration.current.screenHeightDp.dp - keyboardHeight
    val budget = (viewportHeight * .30f).coerceIn(112.dp, 220.dp)
    val columns = if (videos.size == 1) 1 else 2
    val rows = (videos.size + columns - 1) / columns
    val motion = callMotionEnabled()
    BoxWithConstraints(
        Modifier.fillMaxWidth()
            .animateContentSize(
                if (motion) spring(dampingRatio = .9f, stiffness = 500f) else snap()
            )
            .padding(horizontal = 8.dp)
    ) {
        val naturalHeight = (maxWidth - 6.dp * (columns - 1)) / columns * 9f / 16f
        val tileHeight =
            minOf(naturalHeight, (budget - 6.dp * (rows - 1)) / rows).coerceAtLeast(48.dp)
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            videos.chunked(columns).forEach { row ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    row.forEach { video ->
                        key(video.id) {
                            MinimizedCallVideoTile(
                                video,
                                Modifier.weight(1f).height(tileHeight),
                                onExpand,
                            )
                        }
                    }
                    if (row.size < columns) Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun MinimizedCallVideoTile(
    video: CallVideoPreview,
    modifier: Modifier,
    onExpand: () -> Unit,
) {
    val shape = RoundedCornerShape(14.dp)
    Box(
        modifier
            .clip(shape)
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .then(
                if (video.person.speaking)
                    Modifier.border(2.dp, MaterialTheme.colorScheme.primary, shape)
                else Modifier
            )
            .clickable(onClickLabel = ui(R.string.call_expand_video), onClick = onExpand)
            .testTag("call-mini-video-${video.id}")
    ) {
        ManagedAsyncImage(
            if (video.screen) "file:///android_asset/demo/landscape.png"
            else "file:///android_asset/demo/portrait.png",
            ui(if (video.screen) R.string.call_sample_screen else R.string.call_sample_video),
            MediaImages.loader(LocalContext.current),
            Modifier.fillMaxSize(),
            if (video.screen) ContentScale.Fit else ContentScale.Crop,
        )
        Box(
            Modifier.fillMaxSize()
                .background(
                    Brush.verticalGradient(
                        listOf(Color.Transparent, Color.Black.copy(alpha = .85f))
                    )
                )
        )
        Text(
            if (video.person.mine) ui(R.string.you) else video.person.name,
            Modifier.align(Alignment.BottomStart).padding(8.dp),
            color = Color.White,
            style = MaterialTheme.typography.labelSmall,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** Hosted once above navigation, so changing chat or server cannot duplicate the picker. */
@Composable
internal fun CallVideoLimitPicker(calls: VoiceCallContext) {
    val session = calls.state.session ?: return
    if (calls.state.videoLimitPicker != session.sessionId) return
    ModalBottomSheet(onDismissRequest = { calls.controller.dismissVideoLimit(session) }) {
        Column(
            Modifier.fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 24.dp)
                .padding(bottom = 24.dp)
                .selectableGroup()
        ) {
            Text(ui(R.string.call_choose_video_count), style = MaterialTheme.typography.titleLarge)
            Text(
                ui(R.string.call_video_count_description),
                Modifier.padding(vertical = 12.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            (0..4).forEach { limit ->
                Row(
                    Modifier.fillMaxWidth()
                        .heightIn(min = 56.dp)
                        .selectable(
                            selected = session.videoLimit == limit,
                            role = Role.RadioButton,
                        ) {
                            calls.controller.videoLimit(session, limit)
                            calls.controller.dismissVideoLimit(session)
                        }
                        .testTag("call-video-limit-$limit"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = session.videoLimit == limit, onClick = null)
                    Spacer(Modifier.width(12.dp))
                    Text(
                        if (limit == 0) ui(R.string.call_hide_videos)
                        else uiQuantity(R.plurals.call_video_count, limit)
                    )
                }
            }
        }
    }
}
