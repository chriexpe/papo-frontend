package io.github.chriexpe.papo.compose

import android.content.Context
import io.github.chriexpe.papo.core.importLegacyAndroid
import org.json.JSONArray

/** All runtimes, including cold workers, select the same existing cache. */
internal fun messageDatabasePath(context:Context):String {
    val preferences=context.getSharedPreferences("compose-client",0)
    preferences.getString("message-database",null)?.let {return it}
    val compose=context.filesDir.resolve("compose-messages.db")
    val legacy=context.filesDir.resolve("data/papo-cache.db")
    val path=(if(!compose.exists() && legacy.exists()) legacy else compose).absolutePath
    check(preferences.edit().putString("message-database",path).commit()) {ui(R.string.could_not_select_the_message_cache)}
    return path
}

/** The official package keeps its data when replacing the egui APK. Import
 * once, merge rather than overwrite, and leave source files intact on error. */
internal fun importPreviousAndroid(context:Context,credentials:AndroidSecrets) {
    val client=context.getSharedPreferences("compose-client",0)
    if(client.getBoolean("legacy-android-imported",false)) return
    val imported=importLegacyAndroid(context.filesDir.absolutePath,credentials)
    if(imported!=null) {
        val saved=JSONArray(client.getString("servers","[]"))
        val urls=(0 until saved.length()).map {saved.getString(it)}.toMutableList()
        imported.servers.forEach {if(it !in urls) urls+=it}
        val edit=client.edit().putString("servers",JSONArray(urls).toString())
        if(client.getString("active-server","").isNullOrBlank()) edit.putString("active-server",imported.active)
        check(edit.commit()) {ui(R.string.could_not_import_previous_servers)}
        val appearance=context.getSharedPreferences("compose-appearance",0)
        val preferences=appearance.edit()
        if(!appearance.contains("theme")) preferences.putString("theme",imported.theme)
        mapOf("glass" to imported.glass,"notifications-enabled" to imported.notifications,"open-newest" to imported.openNewest).forEach {(key,value)->if(!appearance.contains(key)) preferences.putBoolean(key,value)}
        check(preferences.commit()) {ui(R.string.could_not_import_previous_preferences)}
        androidx.work.WorkManager.getInstance(context).cancelAllWorkByTag("papo-reconcile")
    }
    messageDatabasePath(context)
    check(client.edit().putBoolean("legacy-android-imported",true).commit()) {ui(R.string.could_not_complete_android_migration)}
}
