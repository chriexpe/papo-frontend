package io.github.chriexpe.papo.compose

import android.content.SharedPreferences
import androidx.compose.runtime.*

/** Device preferences have one observable source; account data stays in core. */
@Stable
internal class ClientPreferences(private val storage:SharedPreferences?=null) {
    private var values by mutableStateOf(storage?.all.orEmpty().toMap())
    private val listener=SharedPreferences.OnSharedPreferenceChangeListener {_,_->values=storage?.all.orEmpty().toMap()}
    init {storage?.registerOnSharedPreferenceChangeListener(listener)}
    fun close() {storage?.unregisterOnSharedPreferenceChangeListener(listener)}
    fun boolean(key:String,default:Boolean=true)=values[key] as? Boolean ?: default
    fun string(key:String,default:String="")=values[key] as? String ?: default
    fun set(key:String,value:Boolean) {storage?.edit()?.putBoolean(key,value)?.apply();values=values+(key to value)}
    fun set(key:String,value:String) {storage?.edit()?.putString(key,value)?.apply();values=values+(key to value)}
    val theme get()=string("theme","System")
    val glass get()=boolean("glass")
    val avatars get()=boolean("avatars")
    val timestamps get()=boolean("timestamps")
    val animations get()=boolean("animations")
    val compact get()=boolean("compact",false)
    val catchUpHourglass get()=boolean("catch-up-hourglass")
    val openNewest get()=boolean("open-newest")
    val showDescription get()=boolean("description",false)
}
internal val LocalClientPreferences=compositionLocalOf {ClientPreferences()}
