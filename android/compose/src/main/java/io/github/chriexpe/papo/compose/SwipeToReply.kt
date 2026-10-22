package io.github.chriexpe.papo.compose

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Reply
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.AbsoluteAlignment
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import kotlin.math.abs

/** Horizontal intent must win touch slop before taking over. Child scrubbers
 * and vertical scrolling can consume the gesture first. Only release commits. */
@Composable
internal fun SwipeToReply(id:String,enabled:Boolean,onReply:()->Unit,content:@Composable ()->Unit) {
    val threshold=with(LocalDensity.current) {64.dp.toPx()}
    val limit=with(LocalDensity.current) {88.dp.toPx()}
    val haptics=LocalHapticFeedback.current
    val reply by rememberUpdatedState(onReply)
    var offset by remember(id) {mutableFloatStateOf(0f)}
    var dragging by remember(id) {mutableStateOf(false)}
    var notified by remember(id) {mutableStateOf(false)}
    val position by animateFloatAsState(offset,if(dragging) snap() else tween(180),label=ui(R.string.reply_swipe))
    val progress=(abs(position)/threshold).coerceIn(0f,1f)
    Box(Modifier.fillMaxWidth().clipToBounds().pointerInput(id,enabled,threshold,limit) {
        try {
            if(enabled) detectHorizontalDragGestures(
                onDragStart={dragging=true;notified=false},
                onDragCancel={dragging=false;offset=0f},
                onDragEnd={
                    val committed=offset<=-threshold
                    dragging=false;offset=0f
                    if(committed) reply()
                },
                onHorizontalDrag={change,delta ->
                    offset=(offset+delta).coerceIn(-limit,0f)
                    if(offset<0f) change.consume()
                    if(offset<=-threshold && !notified) {notified=true;haptics.performHapticFeedback(HapticFeedbackType.LongPress)}
                },
            )
        } finally {dragging=false;offset=0f}
    }) {
        if(progress>0f) Surface(shape=CircleShape,color=MaterialTheme.colorScheme.primaryContainer,
            modifier=Modifier.align(AbsoluteAlignment.CenterRight).absolutePadding(right=24.dp).size(36.dp).graphicsLayer {alpha=progress;scaleX=.75f+.25f*progress;scaleY=scaleX}) {
            Box(contentAlignment=Alignment.Center) {Icon(Icons.Outlined.Reply,null,Modifier.size(20.dp),tint=MaterialTheme.colorScheme.primary)}
        }
        Box(Modifier.fillMaxWidth().graphicsLayer {translationX=position}) {content()}
    }
}
