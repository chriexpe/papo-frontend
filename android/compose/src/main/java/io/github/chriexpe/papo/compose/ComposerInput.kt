package io.github.chriexpe.papo.compose

import android.graphics.ImageDecoder
import android.graphics.drawable.Animatable
import android.graphics.drawable.Drawable
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.input.key.*
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.*
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import io.github.chriexpe.papo.core.ComposerMarkdownSpan
import io.github.chriexpe.papo.core.EmojiView
import java.nio.ByteBuffer
import kotlin.math.roundToInt
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal data class ComposerSticker(val range: IntRange, val id: String, val name: String)

private val composerEveryone = Regex("(?i)(?<![\\p{L}\\p{N}_<])@everyone\\b")

internal fun composerStickers(
    text: String,
    emojis: List<EmojiView>,
    markdown: List<ComposerMarkdownSpan> = emptyList(),
): List<ComposerSticker> {
    val byName = emojis.associateBy { it.name }
    return Regex(":([A-Za-z0-9_+\\-]+):")
        .findAll(text)
        .mapNotNull { m ->
            byName[m.groupValues[1]]?.let { ComposerSticker(m.range, it.id, it.name) }
        }
        .filter { sticker ->
            markdown.none { span ->
                (span.kind == "code" || span.kind == "code-block") &&
                    span.start.toInt() <= sticker.range.first &&
                    span.end.toInt() > sticker.range.last
            }
        }
        .toList()
}

internal data class StickerPosition(val offset: Int, val id: String, val name: String)

internal class ComposerProjection(
    draft: ComposerDraft,
    stickers: List<ComposerSticker>,
    loaded: Set<String>,
    color: Color,
    highlightEveryone: Boolean = false,
    markdown: List<ComposerMarkdownSpan> = composerMarkdownSpans(draft),
    typography: Typography = Typography(),
    codeBackground: Color = Color.Transparent,
    quoteColor: Color = color,
) : VisualTransformation {
    private val forward = IntArray(draft.text.length + 1)
    private val reverse = mutableListOf(0)
    val positions = mutableListOf<StickerPosition>()
    val annotated: AnnotatedString

    init {
        val starts = stickers.filter { it.id in loaded }.associateBy { it.range.first }
        val plain = StringBuilder()
        var index = 0
        while (index < draft.text.length) {
            val sticker = starts[index]
            val offset = plain.length
            forward[index] = offset
            if (sticker != null) {
                plain.append(
                    '\u3000'
                ) // one em-wide slot; the real drawable uses this glyph's measured bounds
                positions += StickerPosition(offset, sticker.id, sticker.name)
                for (i in index + 1..sticker.range.last + 1) forward[i] = offset + 1
                index = sticker.range.last + 1
            } else {
                plain.append(draft.text[index])
                index++
                forward[index] = offset + 1
            }
            reverse += index
        }
        annotated = buildAnnotatedString {
            append(plain.toString())
            addComposerMarkdown(
                markdown,
                { forward[it.coerceIn(forward.indices)] },
                typography,
                color,
                codeBackground,
                quoteColor,
            )
            positions.forEach { addStyle(SpanStyle(fontSize = 28.sp), it.offset, it.offset + 1) }
            if (highlightEveryone)
                composerEveryone.findAll(draft.text).forEach {
                    addStyle(
                        SpanStyle(color = color, fontWeight = FontWeight.Bold),
                        forward[it.range.first],
                        forward[it.range.last + 1],
                    )
                }
            draft.mentions
                .filter { it.valid(draft.text) }
                .forEach {
                    addStyle(
                        SpanStyle(color = color, fontWeight = FontWeight.Bold),
                        forward[it.start],
                        forward[it.end],
                    )
                }
        }
    }

    val mapping =
        object : OffsetMapping {
            override fun originalToTransformed(offset: Int) =
                forward[offset.coerceIn(forward.indices)]

            override fun transformedToOriginal(offset: Int) =
                reverse[offset.coerceIn(reverse.indices)]
        }

    override fun filter(text: AnnotatedString) = TransformedText(annotated, mapping)
}

private data class LoadedComposerStickers(
    val owner: String,
    val size: Int,
    val images: Map<String, Drawable>,
)

/**
 * Keep Compose's native IME, selection and cursor. The visual transformation reserves a measured
 * glyph slot, and animated drawables paint in those exact text-layout bounds, so wrapping and
 * scrolling share the editor's layout.
 */
@Composable
internal fun ComposerInput(
    draft: ComposerDraft,
    server: ServerState,
    model: PapoViewModel,
    placeholder: String,
    onChange: (ComposerDraft) -> Unit,
    modifier: Modifier = Modifier,
    onFocusChange: (Boolean) -> Unit = {},
    animationsAllowed: Boolean = true,
    inputTag: String = "composer-input",
    enabled: Boolean = true,
    channelId: String = server.selected,
    onSubmit: (() -> Unit)? = null,
    markdown: List<ComposerMarkdownSpan>? = null,
) {
    val dimensions = LocalUiDimensions.current
    val formatting = remember(server.url, server.me, server.selected) { ComposerFormatting() }
    DisposableEffect(formatting) { onDispose { formatting.close() } }
    SideEffect { formatting.sync(draft) }
    val preview = markdown ?: rememberComposerMarkdown(draft)
    val stickers =
        remember(draft.text, server.emojis, preview) {
            composerStickers(draft.text, server.emojis, preview)
        }
    val ids = remember(stickers) { stickers.map { it.id }.toSet() }
    val owner = "${server.url}/${server.me}/${server.emojis.joinToString {it.revision}}"
    val pixels = with(LocalDensity.current) { 28.sp.toPx().roundToInt().coerceAtLeast(1) }
    val loaded by
        produceState(LoadedComposerStickers(owner, pixels, emptyMap()), owner, ids, pixels) {
            val retained =
                if (value.owner == owner && value.size == pixels)
                    value.images.filterKeys { it in ids }
                else emptyMap()
            value = LoadedComposerStickers(owner, pixels, retained)
            for (id in ids - retained.keys) {
                try {
                    val bytes = model.emojiImage(server, id)
                    val drawable =
                        withContext(Dispatchers.IO) {
                            ImageDecoder.decodeDrawable(
                                ImageDecoder.createSource(ByteBuffer.wrap(bytes))
                            ) { decoder, info, _ ->
                                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                                val scale =
                                    pixels.toFloat() /
                                        maxOf(info.size.width, info.size.height).coerceAtLeast(1)
                                decoder.setTargetSize(
                                    (info.size.width * scale).roundToInt().coerceAtLeast(1),
                                    (info.size.height * scale).roundToInt().coerceAtLeast(1),
                                )
                            }
                        }
                    value = value.copy(images = value.images + (id to drawable))
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    /* Keep the editable :name: text when decoding is unavailable. */
                }
            }
        }
    val images = if (loaded.owner == owner && loaded.size == pixels) loaded.images else emptyMap()
    val color = MaterialTheme.colorScheme.primary
    val highlightEveryone = server.canSuggestEveryone(channelId)
    val typography = MaterialTheme.typography
    val codeBackground = MaterialTheme.colorScheme.surfaceContainerHighest
    val quoteColor = MaterialTheme.colorScheme.onSurfaceVariant
    val projection =
        remember(
            draft.text,
            draft.mentions,
            stickers,
            images.keys,
            color,
            highlightEveryone,
            preview,
            typography,
            codeBackground,
            quoteColor,
        ) {
            ComposerProjection(
                draft,
                stickers,
                images.keys,
                color,
                highlightEveryone,
                preview,
                typography,
                codeBackground,
                quoteColor,
            )
        }
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    var frame by remember { mutableIntStateOf(0) }
    val editorScroll = rememberScrollState()
    var editorHeight by remember { mutableIntStateOf(0) }
    val visibleStickers by
        remember(projection, layout) {
            derivedStateOf {
                val result = layout
                if (result == null || result.layoutInput.text.text != projection.annotated.text)
                    emptySet()
                else
                    projection.positions
                        .filter { position ->
                            val bounds = result.getBoundingBox(position.offset)
                            bounds.bottom > editorScroll.value &&
                                bounds.top < editorScroll.value + editorHeight
                        }
                        .map { it.id }
                        .toSet()
            }
        }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(images, lifecycle, visibleStickers, animationsAllowed) {
        val handler = Handler(Looper.getMainLooper())
        val callback =
            object : Drawable.Callback {
                override fun invalidateDrawable(who: Drawable) {
                    frame++
                }

                override fun scheduleDrawable(who: Drawable, what: Runnable, `when`: Long) {
                    handler.postAtTime(what, who, `when`)
                }

                override fun unscheduleDrawable(who: Drawable, what: Runnable) {
                    handler.removeCallbacks(what, who)
                }
            }
        fun active() {
            images.forEach { (id, d) ->
                d.callback = callback
                (d as? Animatable)?.let {
                    if (
                        animationsAllowed &&
                            id in visibleStickers &&
                            lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
                    )
                        it.start()
                    else it.stop()
                }
            }
        }
        val observer = LifecycleEventObserver { _, _ -> active() }
        lifecycle.addObserver(observer)
        active()
        onDispose {
            lifecycle.removeObserver(observer)
            images.values.forEach { d ->
                (d as? Animatable)?.stop()
                d.callback = null
                handler.removeCallbacksAndMessages(d)
            }
        }
    }
    Box(
        modifier
            .padding(
                start = 14.dp,
                end = 8.dp,
                top = dimensions.composerTextVertical,
                bottom = dimensions.composerTextVertical,
            )
            .heightIn(min = 24.dp, max = 160.dp)
            .onSizeChanged { editorHeight = it.height }
            .verticalScroll(editorScroll)
    ) {
        BasicTextField(
            draft.value,
            enabled = enabled,
            onValueChange = { next ->
                onChange(
                    formatting.edit(
                        draft,
                        next,
                        stickers.filter { it.id in images }.map { it.range },
                    )
                )
            },
            modifier =
                Modifier.fillMaxWidth()
                    .onPreviewKeyEvent { event ->
                        if (
                            onSubmit != null &&
                                (event.key == Key.Enter || event.key == Key.NumPadEnter) &&
                                !event.isShiftPressed &&
                                !event.isCtrlPressed &&
                                !event.isAltPressed &&
                                !event.isMetaPressed &&
                                draft.value.composition == null
                        ) {
                            // Consume the pair, but send only once. Enter used to confirm an
                            // IME candidate must continue through the editor normally.
                            if (
                                event.type == KeyEventType.KeyDown &&
                                    event.nativeKeyEvent.repeatCount == 0
                            )
                                onSubmit()
                            true
                        } else if (
                            event.key == Key.Tab &&
                                event.type == KeyEventType.KeyDown &&
                                !event.isShiftPressed &&
                                !event.isCtrlPressed &&
                                !event.isAltPressed
                        ) {
                            formatting.skip(draft)?.let {
                                onChange(it)
                                true
                            } ?: false
                        } else false
                    }
                    .onFocusChanged { onFocusChange(it.isFocused) }
                    .testTag(inputTag)
                    .semantics {
                        if (projection.positions.isNotEmpty())
                            stateDescription =
                                ui(
                                    R.string.contains_stickers_1_s,
                                    projection.positions.map { it.name }.distinct().joinToString(),
                                )
                    }
                    .drawWithContent {
                        frame // AnimatedImageDrawable invalidates this draw without recomposing the
                        // editor.
                        drawContent()
                        val result = layout
                        if (
                            result != null &&
                                result.layoutInput.text.text == projection.annotated.text
                        ) {
                            projection.positions.forEach { position ->
                                val drawable = images[position.id] ?: return@forEach
                                val bounds = result.getBoundingBox(position.offset)
                                val side = minOf(bounds.width, bounds.height)
                                val ratio =
                                    drawable.intrinsicWidth.toFloat() /
                                        drawable.intrinsicHeight.coerceAtLeast(1)
                                val width = if (ratio >= 1) side else side * ratio
                                val height = if (ratio >= 1) side / ratio else side
                                val left = (bounds.center.x - width / 2).roundToInt()
                                val top = (bounds.center.y - height / 2).roundToInt()
                                drawable.setBounds(
                                    left,
                                    top,
                                    left + width.roundToInt(),
                                    top + height.roundToInt(),
                                )
                                drawable.draw(drawContext.canvas.nativeCanvas)
                            }
                        }
                    },
            textStyle =
                MaterialTheme.typography.bodyLarge.copy(
                    color = MaterialTheme.colorScheme.onSurface,
                    lineHeight = if (projection.positions.isEmpty()) 24.sp else 32.sp,
                    // Allow a larger heading font to grow its line without adding paragraph
                    // boundaries or extra space to the draft's other lines.
                    lineHeightStyle =
                        if (preview.any { it.kind == "heading" })
                            LineHeightStyle(
                                LineHeightStyle.Alignment.Proportional,
                                LineHeightStyle.Trim.None,
                                LineHeightStyle.Mode.Minimum,
                            )
                        else MaterialTheme.typography.bodyLarge.lineHeightStyle,
                ),
            cursorBrush = SolidColor(color),
            visualTransformation = projection,
            onTextLayout = { layout = it },
            keyboardOptions =
                KeyboardOptions(
                    capitalization = KeyboardCapitalization.Sentences,
                    imeAction = if (onSubmit == null) ImeAction.Default else ImeAction.Send,
                ),
            keyboardActions = KeyboardActions(onSend = { onSubmit?.invoke() }),
            decorationBox = { inner ->
                Box {
                    if (draft.text.isEmpty())
                        Text(
                            placeholder,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyLarge,
                            maxLines = 1,
                            softWrap = false,
                            overflow = TextOverflow.Ellipsis,
                        )
                    inner()
                }
            },
        )
    }
}
