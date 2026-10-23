package io.github.chriexpe.papo.compose

import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import io.github.chriexpe.papo.core.MentionSelection
import org.json.JSONArray
import org.json.JSONObject

internal data class ComposerMention(
    val start: Int,
    val label: String,
    val userId: String,
    val allowAdjacent: Boolean = false,
) {
    val end
        get() = start + label.length + 1

    fun valid(text: String) =
        start >= 0 &&
            end <= text.length &&
            text.substring(start, end) == "@$label" &&
            (allowAdjacent ||
                (start == 0 || mentionBoundary(text[start - 1])) &&
                    (end == text.length || mentionBoundary(text[end])))

    // Compose selections and saved bindings use UTF-16 offsets; core expects Unicode code points.
    // Convert only at the bridge boundary so emoji before a mention cannot shift its wire offset.
    fun native(text: String) =
        MentionSelection(text.codePointCount(0, start).toUInt(), label, userId)
}

internal fun mentionBoundary(c: Char) = !c.isLetterOrDigit() && c != '_' && c != '<'

internal data class ComposerTrigger(
    val start: Int,
    val end: Int,
    val query: String,
    val people: Boolean,
)

internal data class ComposerDraft(
    val value: TextFieldValue = TextFieldValue(""),
    val mentions: List<ComposerMention> = emptyList(),
) {
    val text
        get() = value.text

    fun json() =
        JSONArray()
            .apply {
                mentions.forEach {
                    put(
                        JSONObject()
                            .put("start", it.start)
                            .put("label", it.label)
                            .put("id", it.userId)
                            .put("adjacent", it.allowAdjacent)
                    )
                }
            }
            .toString()

    fun edited(next: TextFieldValue): ComposerDraft {
        if (next.text == text) return copy(value = next)
        var prefix = 0
        while (
            prefix < text.length && prefix < next.text.length && text[prefix] == next.text[prefix]
        ) prefix++
        var suffix = 0
        while (
            suffix < text.length - prefix &&
                suffix < next.text.length - prefix &&
                text[text.lastIndex - suffix] == next.text[next.text.lastIndex - suffix]
        ) suffix++
        var oldEnd = text.length - suffix
        // Text comparison alone cannot distinguish deleting the first of two
        // equal labels. Prefer the actual selection/caret edit when it matches.
        val selected = value.selection
        val delta = next.text.length - text.length
        val candidates = mutableListOf(selected.min to selected.max)
        if (selected.collapsed && delta < 0) {
            candidates.add((selected.start + delta).coerceAtLeast(0) to selected.start)
            candidates.add(selected.start to (selected.start - delta).coerceAtMost(text.length))
        }
        candidates
            .firstOrNull { (start, end) ->
                val inserted = next.text.length - (text.length - (end - start))
                inserted >= 0 &&
                    start + inserted <= next.text.length &&
                    text.replaceRange(start, end, next.text.substring(start, start + inserted)) ==
                        next.text
            }
            ?.let { (start, end) ->
                prefix = start
                oldEnd = end
            }
        val retained = mentions.mapNotNull { m ->
            val moved =
                when {
                    m.end <= prefix -> m
                    m.start >= oldEnd -> m.copy(start = m.start + delta)
                    else -> null
                }
            moved?.takeIf { it.valid(next.text) }
        }
        return ComposerDraft(next, retained)
    }

    fun trigger(): ComposerTrigger? {
        if (!value.selection.collapsed) return null
        val cursor = value.selection.end
        if (mentions.any { cursor > it.start && cursor <= it.end + 1 }) return null
        val lineStart = text.lastIndexOf('\n', (cursor - 1).coerceAtLeast(0)) + 1
        val at = text.lastIndexOf('@', cursor - 1)
        val colon = text.lastIndexOf(':', cursor - 1)
        val start = maxOf(at, colon)
        if (
            start < lineStart ||
                start < 0 ||
                cursor - start > 80 ||
                (start > 0 && !mentionBoundary(text[start - 1]))
        )
            return null
        val query = text.substring(start + 1, cursor)
        val people = start == at
        if (
            query.contains('\n') ||
                query.contains('@') ||
                query.contains(':') ||
                (!people && query.any { it.isWhitespace() })
        )
            return null
        var end = cursor
        while (end < text.length && (text[end].isLetterOrDigit() || text[end] in "_+-")) end++
        if (!people && end < text.length && text[end] == ':') end++
        return ComposerTrigger(start, end, query, people)
    }

    fun insert(
        token: String,
        trigger: ComposerTrigger? = null,
        person: Person? = null,
    ): ComposerDraft {
        val start = trigger?.start ?: value.selection.min
        var end = trigger?.end ?: value.selection.max
        while (end < text.length && (text[end] == ' ' || text[end] == '\t')) end++
        val next = text.replaceRange(start, end, "$token ")
        val edited = edited(TextFieldValue(next, TextRange(start + token.length + 1)))
        return if (person == null) edited
        else
            edited.copy(mentions = edited.mentions + ComposerMention(start, person.name, person.id))
    }

    fun mention(person: Person): ComposerDraft {
        val start = value.selection.min
        val spaced =
            if (start > 0 && !text[start - 1].isWhitespace()) {
                edited(
                    TextFieldValue(
                        text.replaceRange(start, value.selection.max, " "),
                        TextRange(start + 1),
                    )
                )
            } else this
        return spaced.insert("@${person.name}", person = person)
    }

    fun deleteSticker(next: TextFieldValue, stickers: List<IntRange>): ComposerDraft {
        if (value.selection.collapsed && next.text.length == text.length - 1) {
            val cursor = value.selection.start
            val range = stickers.firstOrNull { r ->
                (cursor == r.last + 1 && next.text == text.removeRange(cursor - 1, cursor)) ||
                    (cursor == r.first && next.text == text.removeRange(cursor, cursor + 1))
            }
            if (range != null)
                return edited(
                    TextFieldValue(
                        text.removeRange(range.first, range.last + 1),
                        TextRange(range.first),
                    )
                )
        }
        return edited(next)
    }

    companion object {
        fun fromEditable(message: io.github.chriexpe.papo.core.EditableMessage): ComposerDraft {
            val bindings =
                message.mentions.map { mention ->
                    ComposerMention(
                        message.text.offsetByCodePoints(0, mention.start.toInt()),
                        mention.label,
                        mention.userId,
                        allowAdjacent = true,
                    )
                }
            return ComposerDraft(
                TextFieldValue(message.text, TextRange(message.text.length)),
                bindings,
            )
        }

        fun restore(text: String, json: String, selection: Int = text.length): ComposerDraft {
            val bindings = runCatching {
                val a = JSONArray(json)
                (0 until a.length()).map {
                    val m = a.getJSONObject(it)
                    ComposerMention(
                        m.getInt("start"),
                        m.getString("label"),
                        m.getString("id"),
                        m.optBoolean("adjacent"),
                    )
                }
            }
                .getOrDefault(emptyList())
            return ComposerDraft(
                TextFieldValue(text, TextRange(selection.coerceIn(0, text.length))),
                bindings.filter { it.valid(text) },
            )
        }

        val Saver: Saver<ComposerDraft, Any> =
            listSaver(
                save = {
                    listOf(it.text, it.json(), it.value.selection.start, it.value.selection.end)
                },
                restore = {
                    restore(it[0] as String, it[1] as String)
                        .copy(
                            value =
                                TextFieldValue(
                                    it[0] as String,
                                    TextRange(it[2] as Int, it[3] as Int),
                                )
                        )
                },
            )
    }
}
