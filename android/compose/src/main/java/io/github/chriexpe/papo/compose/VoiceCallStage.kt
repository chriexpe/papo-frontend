package io.github.chriexpe.papo.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ScreenShare
import androidx.compose.material.icons.automirrored.outlined.VolumeOff
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun CallStage(
    people: List<CallParticipant>,
    session: CallPreviewSession,
    focused: String?,
    onFocus: (String?) -> Unit,
    modifier: Modifier,
) {
    val onParticipant = LocalOpenCallParticipant.current
    val selected = people.firstOrNull { it.id == focused }
    when {
        selected != null ->
            Column(modifier.padding(12.dp)) {
                CallParticipantTile(selected, Modifier.weight(1f).fillMaxWidth()) { onFocus(null) }
                FilledTonalButton(
                    onClick = { onFocus(null) },
                    modifier =
                        Modifier.align(Alignment.CenterHorizontally)
                            .padding(top = 12.dp)
                            .heightIn(min = 48.dp)
                            .testTag("call-show-everyone"),
                ) {
                    Icon(Icons.Outlined.GridView, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(ui(R.string.call_show_everyone))
                }
            }
        session.layout == CallPreviewLayout.Screen ->
            Column(
                modifier.padding(horizontal = 12.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Surface(
                    Modifier.weight(1f).fillMaxWidth().testTag("call-screen-share"),
                    shape = RoundedCornerShape(24.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerLow,
                ) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Outlined.ScreenShare,
                            null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text(
                            ui(R.string.call_sample_screen),
                            Modifier.padding(12.dp),
                            style = MaterialTheme.typography.labelMedium,
                        )
                        ManagedAsyncImage(
                            "file:///android_asset/demo/landscape.png",
                            ui(R.string.call_sample_screen),
                            MediaImages.loader(LocalContext.current),
                            Modifier.fillMaxWidth().weight(1f),
                            ContentScale.Fit,
                        )
                    }
                }
                LazyRow(
                    contentPadding = PaddingValues(vertical = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    items(people, key = { it.id }) { person ->
                        Column(
                            Modifier.width(80.dp)
                                .clip(RoundedCornerShape(16.dp))
                                .clickable(
                                    onClickLabel = ui(R.string.call_open_participant, person.name)
                                ) {
                                    onParticipant(person.id)
                                }
                                .padding(8.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            CallParticipantAvatar(person, 44)
                            Text(
                                person.name,
                                Modifier.padding(top = 6.dp),
                                style = MaterialTheme.typography.labelSmall,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
        session.layout == CallPreviewLayout.Video && people.size == 2 ->
            Box(modifier.padding(12.dp)) {
                val remote = people.first { !it.mine }
                val local = people.first { it.mine }
                CallParticipantTile(remote, Modifier.fillMaxSize()) { onFocus(remote.id) }
                CallParticipantTile(
                    local,
                    Modifier.align(Alignment.TopEnd).padding(12.dp).width(108.dp).aspectRatio(.8f),
                ) {
                    onFocus(local.id)
                }
            }
        else ->
            LazyVerticalGrid(
                columns = GridCells.Adaptive(140.dp),
                modifier = modifier.fillMaxWidth().testTag("call-participant-grid"),
                contentPadding = PaddingValues(12.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                items(people, key = { it.id }) { person ->
                    CallParticipantTile(
                        person,
                        Modifier.fillMaxWidth()
                            .aspectRatio(
                                if (session.layout == CallPreviewLayout.Video) .85f else 1f
                            ),
                    ) {
                        onFocus(person.id)
                    }
                }
            }
    }
}

@Composable
private fun CallParticipantTile(person: CallParticipant, modifier: Modifier, onFocus: () -> Unit) {
    val shape = RoundedCornerShape(24.dp)
    val onParticipant = LocalOpenCallParticipant.current
    Box(
        modifier
            .clip(shape)
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .then(
                if (person.speaking) Modifier.border(2.dp, MaterialTheme.colorScheme.primary, shape)
                else Modifier
            )
            .clickable(
                onClickLabel = ui(R.string.call_focus_person, person.name),
                onClick = onFocus,
            )
            .testTag("call-participant-${person.id}")
            .semantics {
                stateDescription =
                    when {
                        person.speaking -> ui(R.string.call_person_speaking, person.name)
                        person.muted -> ui(R.string.call_muted)
                        else -> ui(R.string.call_listening)
                    }
            },
        contentAlignment = Alignment.Center,
    ) {
        if (person.camera) {
            ManagedAsyncImage(
                "file:///android_asset/demo/portrait.png",
                null,
                MediaImages.loader(LocalContext.current),
                Modifier.fillMaxSize(),
                ContentScale.Crop,
            )
            Box(
                Modifier.fillMaxSize()
                    .background(
                        Brush.verticalGradient(
                            listOf(Color.Transparent, Color.Black.copy(alpha = .8f))
                        )
                    )
            )
            Text(
                ui(R.string.call_sample_video),
                Modifier.align(Alignment.TopStart)
                    .padding(12.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(Color.Black.copy(alpha = .7f))
                    .padding(6.dp),
                color = Color.White,
                style = MaterialTheme.typography.labelSmall,
            )
        } else {
            Box(
                Modifier.size(64.dp).clip(RoundedCornerShape(22.dp)).clickable(
                    onClickLabel = ui(R.string.call_open_participant, person.name)
                ) {
                    onParticipant(person.id)
                }
            ) {
                CallParticipantAvatar(person, 64)
            }
        }
        Row(
            Modifier.align(Alignment.BottomStart).fillMaxWidth().padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                if (person.mine) ui(R.string.you) else person.name,
                Modifier.weight(1f)
                    .heightIn(min = 48.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .clickable(onClickLabel = ui(R.string.call_open_participant, person.name)) {
                        onParticipant(person.id)
                    }
                    .wrapContentHeight(Alignment.Bottom),
                style = MaterialTheme.typography.labelLarge,
                color = if (person.camera) Color.White else userNameColor(person.id),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (person.audio.muted || person.audio.volumePercent == 0)
                Icon(
                    Icons.AutoMirrored.Outlined.VolumeOff,
                    ui(R.string.call_muted_for_me),
                    Modifier.size(18.dp),
                    tint =
                        if (person.camera) Color.White
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            if (person.muted || person.speaking)
                Icon(
                    if (person.muted) Icons.Outlined.MicOff else Icons.Outlined.GraphicEq,
                    ui(if (person.muted) R.string.call_muted else R.string.call_speaking),
                    Modifier.size(18.dp),
                    tint =
                        if (person.camera) Color.White
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                )
        }
    }
}

@Composable
private fun CallParticipantAvatar(person: CallParticipant, size: Int) {
    UserAvatar(person.id, person.name, Modifier.size(size.dp), RoundedCornerShape((size / 3).dp))
}
