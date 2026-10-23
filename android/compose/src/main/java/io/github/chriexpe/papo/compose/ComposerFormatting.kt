package io.github.chriexpe.papo.compose

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import io.github.chriexpe.papo.core.FormatAssistant
import io.github.chriexpe.papo.core.FormatEdit

/**
 * Only actual collapsed keyboard edits reach core. IME composition, selected replacements, paste
 * and programmatic mention/sticker inserts keep native text semantics. Apply the user's edit first
 * to retain bound mention identities.
 */
internal class ComposerFormatting : AutoCloseable {
    private val assistant = FormatAssistant()
    private var expected: String? = null

    fun sync(draft: ComposerDraft) {
        if (expected != draft.text) {
            assistant.reset()
            expected = draft.text
        }
    }

    fun edit(draft: ComposerDraft, next: TextFieldValue, stickers: List<IntRange>): ComposerDraft {
        sync(draft)
        val written = draft.deleteSticker(next, stickers)
        val nativeEdit =
            draft.value.selection.collapsed &&
                next.selection.collapsed &&
                written.value == next &&
                draft.text.validUnicode() &&
                next.text.validUnicode()
        val edit =
            when {
                !nativeEdit -> {
                    assistant.reset()
                    null
                }
                draft.value.composition != null || next.composition != null -> {
                    assistant.track(draft.text, next.text)
                    null
                }
                else ->
                    assistant.change(
                        draft.text,
                        next.text,
                        next.text.codePointCount(0, next.selection.end).toUInt(),
                    )
            }
        val result = edit?.let { written.applyFormatting(it) } ?: written
        expected = result.text
        return result
    }

    fun skip(draft: ComposerDraft): ComposerDraft? {
        sync(draft)
        if (
            draft.value.composition != null ||
                !draft.value.selection.collapsed ||
                !draft.text.validUnicode()
        )
            return null
        val edit =
            assistant.skip(
                draft.text,
                draft.text.codePointCount(0, draft.value.selection.end).toUInt(),
            ) ?: return null
        return draft.applyFormatting(edit).also { expected = it.text }
    }

    override fun close() {
        assistant.close()
    }
}

private fun ComposerDraft.applyFormatting(edit: FormatEdit): ComposerDraft {
    val caret =
        edit.text.offsetByCodePoints(
            0,
            edit.caret.toInt().coerceIn(0, edit.text.codePointCount(0, edit.text.length)),
        )
    return edited(TextFieldValue(edit.text, TextRange(caret)))
}

/**
 * A transient half-surrogate must stay with the editor, never cross UniFFI's strict UTF-8 boundary.
 * Normal emoji commits remain a single scalar edit.
 */
private fun String.validUnicode(): Boolean {
    var at = 0
    while (at < length) {
        val c = this[at++]
        if (c.isHighSurrogate()) {
            if (at >= length || !this[at++].isLowSurrogate()) return false
        } else if (c.isLowSurrogate()) return false
    }
    return true
}
