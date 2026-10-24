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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.GridView
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

private data class CallVideoPreview(val person: CallParticipant, val screen: Boolean = false) {
    val id
        get() = "${person.id}/$screen"
}

private fun previewVideos(
    session: CallSession,
    people: List<CallParticipant>,
): List<CallVideoPreview> {
    val screen =
        if (!session.demo) people.filter { it.screen }.map { CallVideoPreview(it, screen = true) }
        else if (session.layout == CallLayout.Screen) {
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
    session: CallSession,
    people: List<CallParticipant>,
) {
    val available = previewVideos(session, people)
    if (available.isEmpty()) return
    val visible = visibleVideos(available, session.videoLimit)
    val motion = callMotionEnabled()
    // The video surface enters/exits as one piece. A separate visibility
    // flag preserves the 1–4 count when closed; hiding never changes publication or membership.
    val visibility = remember(session.sessionId) { MutableTransitionState(false) }
    LaunchedEffect(session.videoPreviewsVisible) {
        visibility.targetState = session.videoPreviewsVisible
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
        Box(Modifier.fillMaxWidth().padding(bottom = 8.dp).testTag("call-minimized-video")) {
            MinimizedVideoGrid(visible, calls.controller::reopen)
            IconButton(
                onClick = { calls.controller.hideVideoPreviews(session) },
                modifier =
                    Modifier.align(Alignment.TopEnd)
                        .padding(end = 12.dp, top = 4.dp)
                        .size(48.dp)
                        .testTag("call-video-close"),
            ) {
                Box(
                    Modifier.size(24.dp)
                        .clip(CircleShape)
                        .background(Color.Black.copy(alpha = .5f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        Icons.Outlined.Close,
                        ui(R.string.call_hide_videos),
                        Modifier.size(16.dp),
                        tint = Color.White.copy(alpha = .9f),
                    )
                }
            }
        }
    }
}

/**
 * A normal header control beside the participant strip; it exists only with an open video panel.
 */
@Composable
internal fun CallVideoCountButton(calls: VoiceCallContext, session: CallSession) {
    IconButton(
        onClick = { calls.controller.cycleVideoLimit(session) },
        modifier =
            Modifier.size(48.dp).testTag("call-video-count").semantics {
                contentDescription =
                    ui(
                        R.string.call_cycle_video_count,
                        uiQuantity(R.plurals.call_video_count, session.videoLimit % 4 + 1),
                    )
                stateDescription = uiQuantity(R.plurals.call_video_count, session.videoLimit)
            },
    ) {
        Row(
            Modifier.size(38.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Icon(Icons.Outlined.GridView, null, Modifier.size(18.dp))
            Spacer(Modifier.width(3.dp))
            Text(session.videoLimit.toString(), style = MaterialTheme.typography.labelSmall)
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
        CallVideoImage(video.person.id, video.screen, Modifier.fillMaxSize())
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
