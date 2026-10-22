package io.github.chriexpe.papo.compose

import android.app.LocaleManager
import android.content.Context
import android.content.res.Configuration
import android.content.res.Resources
import android.os.Build
import android.os.LocaleList
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.ui.semantics.Role
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

internal object AppLanguage {
    private var resources by mutableStateOf<Resources?>(null)
    fun selected(context:Context):String = if(Build.VERSION.SDK_INT>=33) {
        context.getSystemService(LocaleManager::class.java).applicationLocales.toLanguageTags().let {if(it.startsWith("pt")) "pt-BR" else if(it.startsWith("en")) "en" else ""}
    } else context.getSharedPreferences("compose-appearance",0).getString("language","").orEmpty()
    fun context(base:Context):Context {
        val tag=selected(base)
        if(tag.isEmpty()) return base
        val config=Configuration(base.resources.configuration)
        config.setLocales(LocaleList.forLanguageTags(tag))
        return base.createConfigurationContext(config)
    }
    fun update(context:Context) {resources=this.context(context).resources}
    fun choose(context:Context,tag:String) {
        require(tag in setOf("","pt-BR","en"))
        context.getSharedPreferences("compose-appearance",0).edit().putString("language",tag).apply()
        if(Build.VERSION.SDK_INT>=33) context.getSystemService(LocaleManager::class.java).applicationLocales=LocaleList.forLanguageTags(tag)
        else {update(context.applicationContext);(context as? android.app.Activity)?.recreate()}
    }
    val locale get()=(resources ?: Resources.getSystem()).configuration.locales[0]
    fun quantity(id:Int,count:Int):String = (resources ?: Resources.getSystem()).getQuantityString(id,count,count)
    fun text(id:Int,vararg args:Any):String {
        val current=resources ?: Resources.getSystem()
        return current.getString(id,*args)
    }
}

/** Android resources are also available in worker/callback code. Reading the
 * observable Resources here makes Compose labels update with app locale changes. */
internal fun ui(id:Int,vararg args:Any)=AppLanguage.text(id,*args)

@Composable
internal fun LanguageSettings() {
    val context=LocalContext.current
    val configuration=LocalConfiguration.current
    val selected=remember(configuration) {AppLanguage.selected(context)}
    Column(Modifier.fillMaxWidth().padding(20.dp).testTag("language-settings"),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        Text(ui(R.string.choose_the_language_used_by_papo_messages_and_server_content_keep_their_ori))
        SettingsCard {
            Column(Modifier.selectableGroup()) {
            listOf("" to uiLabel("System"),"pt-BR" to "Português (Brasil)","en" to "English").forEach {(tag,label) ->
                ListItem(headlineContent={Text(label)},trailingContent={if(tag==selected) Icon(Icons.Outlined.Check,null)},
                    modifier=Modifier.testTag("language-${tag.ifEmpty {"system"}}").then(Modifier.selectable(selected=tag==selected,role=Role.RadioButton,onClick= {AppLanguage.choose(context,tag)})),
                    colors=ListItemDefaults.colors(containerColor=androidx.compose.ui.graphics.Color.Transparent))
            }
            }
        }
    }
}

/** Labels for stable navigation/preference tokens only; never translate server content. */
internal fun uiLabel(value:String):String = when(value) {
    "Settings" -> ui(R.string.settings)
    "Account" -> ui(R.string.account)
    "Appearance" -> ui(R.string.appearance)
    "Notifications" -> ui(R.string.notifications)
    "Files" -> ui(R.string.files)
    "Sessions" -> ui(R.string.sessions)
    "Diagnostics" -> ui(R.string.diagnostics)
    "Chats" -> ui(R.string.chats)
    "Direct" -> ui(R.string.direct)
    "Servers" -> ui(R.string.servers)
    "People" -> ui(R.string.people)
    "General" -> ui(R.string.general)
    "Channels" -> ui(R.string.channels)
    "Roles" -> ui(R.string.roles_2)
    "Stickers" -> ui(R.string.stickers)
    "Audit" -> ui(R.string.audit)
    "System" -> ui(R.string.system)
    "Light" -> ui(R.string.light)
    "Dark" -> ui(R.string.dark)
    "Online" -> ui(R.string.online)
    "Away" -> ui(R.string.away)
    "Busy" -> ui(R.string.busy)
    "Offline" -> ui(R.string.offline)
    "All" -> ui(R.string.all)
    "Text" -> ui(R.string.text)
    "Voice" -> ui(R.string.voice)
    "Category" -> ui(R.string.category)
    "Favorites" -> ui(R.string.favorites)
    "Recent" -> ui(R.string.recent)
    "Trending" -> ui(R.string.trending)
    "Emoji" -> ui(R.string.emoji)
    "GIFs" -> ui(R.string.gifs)
    "Image" -> ui(R.string.image)
    "Video" -> ui(R.string.video)
    "Audio" -> ui(R.string.audio)
    "Media" -> ui(R.string.media)
    "Message" -> ui(R.string.message)
    "Everyone" -> ui(R.string.everyone)
    "Owner" -> ui(R.string.owner)
    "New channel" -> ui(R.string.new_channel)
    "New category" -> ui(R.string.new_category)
    "New role" -> ui(R.string.new_role)
    "Language" -> ui(R.string.language)
    "Links" -> ui(R.string.links)
    "Username" -> ui(R.string.username)
    "Password" -> ui(R.string.password)
    "Done" -> ui(R.string.done)
    "Back" -> ui(R.string.back)
    "Next" -> ui(R.string.next)
    "Copy" -> ui(R.string.copy)
    "Copied" -> ui(R.string.copied)
    "Favorite" -> ui(R.string.favorite)
    "Favorited" -> ui(R.string.favorited)
    "Reply" -> ui(R.string.reply)
    "Edit" -> ui(R.string.edit)
    "Send" -> ui(R.string.send)
    "Unban" -> ui(R.string.unban)
    "Ban" -> ui(R.string.ban)
    "Square" -> ui(R.string.square)
    "Original" -> ui(R.string.original)
    "Remove" -> ui(R.string.remove)
    "Close" -> ui(R.string.close)
    "Search" -> ui(R.string.search)
    "Today" -> ui(R.string.today)
    "Yesterday" -> ui(R.string.yesterday)
    "Initials" -> ui(R.string.initials)
    "Name" -> ui(R.string.name)
    "Enabled" -> ui(R.string.enabled)
    "Disabled" -> ui(R.string.disabled)
    "On" -> ui(R.string.on)
    "Off" -> ui(R.string.off)
    "roles" -> ui(R.string.roles_3)
    "role" -> ui(R.string.role)
    "Read" -> ui(R.string.read)
    "Write" -> ui(R.string.write)
    "Connect" -> ui(R.string.connect)
    "typing" -> ui(R.string.typing)
    "Camera" -> ui(R.string.camera)
    else -> value
}

internal fun uiQuantity(id:Int,count:Int)=AppLanguage.quantity(id,count)
