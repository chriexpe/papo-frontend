package io.github.chriexpe.papo.compose

import android.content.Context
import android.icu.lang.UCharacter
import android.icu.lang.UProperty
import android.icu.text.BreakIterator
import io.github.chriexpe.papo.core.GifView
import java.io.File
import java.security.MessageDigest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

internal data class EmojiChoice(val value: String, val custom: Boolean = false, val name: String = value) {
    val key get() = "${if (custom) "c" else "u"}:$value"
    val token get() = if (custom) ":$name:" else value
}
internal data class GifChoice(
    val key: String, val title: String, val providerId: String = "", val url: String = "",
    val preview: String = url, val width: Int = 480, val height: Int = 320,
    val attachment: String = "", val embed: String = "", val revision: String = "", val path: String = "",
) {
    companion object {
        fun from(g: GifView) = GifChoice("giphy:${g.id}", g.title, g.id, g.gifUrl, g.previewUrl, g.width.toInt(), g.height.toInt())
        fun marker(text: String): String? = text.trim().removePrefix("giphy:").takeIf {
            text.trim().startsWith("giphy:") && it.matches(Regex("[A-Za-z0-9_-]{1,200}"))
        }
    }
    fun json() = JSONObject().put("key", key).put("title", title).put("provider", providerId).put("url", url)
        .put("preview", preview).put("width", width).put("height", height).put("attachment", attachment)
        .put("embed", embed).put("revision", revision).put("path", path)
}
internal data class ExpressionState(val favorites: List<GifChoice> = emptyList(), val recent: List<GifChoice> = emptyList(), val usage: List<Pair<EmojiChoice, Int>> = emptyList())

/** Private, account-scoped library. Saved attachment/relay GIFs keep an owned
 * copy so a favorite still works if the source message disappears. */
internal class ExpressionLibrary(private val context: Context) {
    private val preferences = context.getSharedPreferences("compose-expressions", Context.MODE_PRIVATE)
    private val mutable = MutableStateFlow<Map<String, ExpressionState>>(emptyMap())
    val states = mutable.asStateFlow()
    fun owner(server: ServerState) = "${server.url}/${server.me}"
    private fun decode(array: JSONArray) = (0 until array.length()).mapNotNull { i -> runCatching {
        val j = array.getJSONObject(i)
        GifChoice(j.getString("key"), j.getString("title"), j.optString("provider"), j.optString("url"), j.optString("preview"), j.optInt("width",480), j.optInt("height",320), j.optString("attachment"), j.optString("embed"), j.optString("revision"), j.optString("path"))
    }.getOrNull() }
    @Synchronized fun state(owner: String): ExpressionState {
        mutable.value[owner]?.let { return it }
        val j = runCatching { JSONObject(preferences.getString(owner, "{}")!!) }.getOrDefault(JSONObject())
        val usage = j.optJSONArray("usage") ?: JSONArray()
        val result = ExpressionState(decode(j.optJSONArray("favorites") ?: JSONArray()), decode(j.optJSONArray("recent") ?: JSONArray()),
            (0 until usage.length()).mapNotNull { i -> runCatching { val e=usage.getJSONObject(i); EmojiChoice(e.getString("value"),e.getBoolean("custom"),e.getString("name")) to e.getInt("count") }.getOrNull() })
        mutable.value = mutable.value + (owner to result)
        return result
    }
    private fun save(owner: String, state: ExpressionState) {
        val j = JSONObject().put("favorites", JSONArray().apply { state.favorites.forEach { put(it.json()) } })
            .put("recent", JSONArray().apply { state.recent.forEach { put(it.json()) } })
            .put("usage",JSONArray().apply { state.usage.forEach { (e,n) -> put(JSONObject().put("value",e.value).put("custom",e.custom).put("name",e.name).put("count",n)) } })
        check(preferences.edit().putString(owner,j.toString()).commit()) { "Couldn’t save your expression library" }
        mutable.value = mutable.value + (owner to state)
    }
    @Synchronized fun used(owner: String, choices: List<EmojiChoice>) {
        val s=state(owner); val usage=s.usage.toMutableList()
        choices.forEach { e -> val old=usage.firstOrNull { it.first.key==e.key }; usage.removeAll { it.first.key==e.key }; usage.add(0,e to ((old?.second ?: 0)+1).coerceAtMost(1_000_000)) }
        save(owner,s.copy(usage=usage.sortedByDescending { it.second }.take(200)))
    }
    @Synchronized fun recent(owner: String, gif: GifChoice) { val s=state(owner); save(owner,s.copy(recent=(listOf(gif)+s.recent.filter { it.key!=gif.key }).take(50))); prune(owner) }
    @Synchronized fun favorite(owner: String, gif: GifChoice) {
        val s=state(owner)
        save(owner,s.copy(favorites=if (s.favorites.any { it.key==gif.key }) s.favorites.filter { it.key!=gif.key } else listOf(gif)+s.favorites))
        prune(owner)
    }
    fun file(owner: String, key: String): File {
        fun hash(value:String)=MessageDigest.getInstance("SHA-256").digest(value.toByteArray()).joinToString("") { "%02x".format(it) }
        return File(File(context.filesDir,"gif-library/${hash(owner)}").apply { mkdirs() }, "${hash(key)}.gif")
    }
    private fun prune(owner: String) {
        val s=state(owner); val keep=(s.favorites+s.recent).map { it.path }.toSet()
        file(owner,"_").parentFile?.listFiles()?.filter { it.absolutePath !in keep && it.lastModified()<System.currentTimeMillis()-86_400_000 }?.forEach { it.delete() }
    }
}

internal fun messageEmoji(text: String, server: ServerState): List<EmojiChoice> {
    val result=mutableListOf<EmojiChoice>()
    val custom=server.emojis.associateBy { it.name }
    val remaining=Regex(":([A-Za-z0-9_+\\-]+):").replace(text) { m -> custom[m.groupValues[1]]?.let { result+=EmojiChoice(it.id,true,it.name); " " } ?: m.value }
    val iterator=BreakIterator.getCharacterInstance().apply { setText(remaining) }
    var start=iterator.first(); var end=iterator.next()
    while (end!=BreakIterator.DONE) {
        val glyph=remaining.substring(start,end)
        if (glyph.codePoints().anyMatch { it>=0x2300 && UCharacter.hasBinaryProperty(it,UProperty.EMOJI) }) result+=EmojiChoice(glyph)
        start=end; end=iterator.next()
    }
    return result
}
