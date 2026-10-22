package io.github.chriexpe.papo.compose

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.work.*
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import io.github.chriexpe.papo.core.MessageAlert
import io.github.chriexpe.papo.core.NotificationListener
import io.github.chriexpe.papo.core.NotificationReconcile
import io.github.chriexpe.papo.notifications.MessageNotifications
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject
import java.util.concurrent.TimeUnit

internal data class NotificationTarget(val server:String,val owner:String,val channel:String,val message:String)

/** One platform adapter for live core events and headless reconciliation.
 * FCM notification payloads are rendered by Android while we are backgrounded. */
internal object MessageAlerts {
    @Volatile private var application:Context?=null
    @Volatile var foreground=false
    val changes=MutableStateFlow(0L)
    val pushHints=kotlinx.coroutines.flow.MutableSharedFlow<String>(extraBufferCapacity=1)
    private val context get()=checkNotNull(application)
    private val preferences get()=context.getSharedPreferences("compose-notifications",0)
    private val appearance get()=context.getSharedPreferences("compose-appearance",0)
    val enabled get()=appearance.getBoolean("notifications-enabled",true)
    val token get()=preferences.getString("push-token","").orEmpty()
    val configured get()=BuildConfig.FIREBASE_CONFIGURED && FirebaseApp.getApps(context).isNotEmpty()
    val listener=object:NotificationListener {
        override fun available()=enabled && MessageNotifications.available(context)
        override fun isForeground()=foreground
        override fun show(alert:MessageAlert) {
            if(!available() || preferences.getString("owner/${alert.serverUrl}","")!=alert.ownerUserId) return
            MessageNotifications.show(context,JSONObject().put("server_url",alert.serverUrl).put("owner_user_id",alert.ownerUserId)
                .put("channel_id",alert.channelId).put("message_id",alert.messageId).put("notification_id",alert.notificationId)
                .put("title",alert.title).put("body",alert.body).toString(),MainActivity::class.java,R.drawable.ic_message_notification)
        }
    }
    private val preferenceListener=android.content.SharedPreferences.OnSharedPreferenceChangeListener {_,key ->if(key=="notifications-enabled") refresh()}
    @Synchronized fun start(value:Context) {
        if(application!=null) return
        application=value.applicationContext
        AppLanguage.update(value)
        localizeChannel()
        appearance.registerOnSharedPreferenceChangeListener(preferenceListener)
        if(preferences.getString("firebase-config",null)!=BuildConfig.FIREBASE_CONFIGURATION) {
            preferences.edit().remove("push-token").putString("firebase-config",BuildConfig.FIREBASE_CONFIGURATION).apply()
        }
        if(configured) FirebaseMessaging.getInstance().token.addOnSuccessListener {newToken(it)}
    }
    private fun localizeChannel() {
        if(application==null) return
        AppLanguage.update(context)
        MessageNotifications.ensureChannel(context,ui(R.string.notification_channel_name),ui(R.string.notification_channel_description))
    }
    fun refresh() {localizeChannel();synchronized(this) {changes.value=changes.value+1}}
    fun newToken(value:String) {
        if(value.isNotBlank() && value!=token) {preferences.edit().putString("push-token",value).apply();refresh()}
    }
    fun owner(server:String)=preferences.getString("owner/$server","").orEmpty()
    fun setOwner(server:String,owner:String) {
        val previous=owner(server)
        if(previous==owner) return
        if(previous.isNotBlank()) clearAccount(server,previous)
        preferences.edit().putString("owner/$server",owner).remove("registered/$server").apply()
    }
    fun registered(server:String,owner:String)=configured && preferences.getString("registered/$server",null)?.let {
        runCatching {val json=JSONObject(it);json.getString("owner")==owner && json.getString("token")==token}.getOrDefault(false)
    }==true
    fun registration(server:String):Pair<String,String>?=preferences.getString("registered/$server",null)?.let {runCatching {val j=JSONObject(it);j.getString("owner") to j.getString("token")}.getOrNull()}
    fun confirm(server:String,owner:String,value:String,enabled:Boolean) {
        if(owner(server)!=owner) return
        val edit=preferences.edit()
        if(enabled) edit.putString("registered/$server",JSONObject().put("owner",owner).put("token",value).toString()) else edit.remove("registered/$server")
        edit.apply()
    }
    fun clear(server:String,owner:String,channel:String) {
        MessageNotifications.clear(context,JSONObject().put("server_url",server).put("owner_user_id",owner).put("channel_id",channel).toString())
    }
    private fun clearAccount(server:String,owner:String) {
        val manager=context.getSystemService(NotificationManager::class.java)
        manager.activeNotifications.filter {it.tag?.startsWith("papo-message:$server:$owner:")==true}.forEach {manager.cancel(it.tag,it.id)}
    }
    fun target(intent:Intent?):NotificationTarget? {
        if(intent==null) return null
        val local=intent.hasExtra(MessageNotifications.EXTRA_MESSAGE_ID)
        if(!local && !intent.hasExtra("google.message_id")) return null
        val target=NotificationTarget(intent.getStringExtra(if(local) MessageNotifications.EXTRA_SERVER else "server_url").orEmpty(),
            intent.getStringExtra(if(local) MessageNotifications.EXTRA_OWNER else "owner_user_id").orEmpty(),
            intent.getStringExtra(if(local) MessageNotifications.EXTRA_CHANNEL else "channel_id").orEmpty(),
            intent.getStringExtra(if(local) MessageNotifications.EXTRA_MESSAGE_ID else "message_id").orEmpty())
        listOf(MessageNotifications.EXTRA_SERVER,MessageNotifications.EXTRA_OWNER,MessageNotifications.EXTRA_CHANNEL,MessageNotifications.EXTRA_MESSAGE_ID,
            MessageNotifications.EXTRA_NOTIFICATION_ID,"google.message_id","server_url","owner_user_id","channel_id","message_id","notification_id").forEach(intent::removeExtra)
        return target.takeIf {it.channel.isNotBlank() && it.message.isNotBlank() && (!local || it.server.isNotBlank() && it.owner.isNotBlank())}
    }
    @Synchronized fun schedule(servers:List<ServerState>) {
        val wanted=servers.filter {!it.isDemo && it.screen=="chat" && it.me.isNotBlank() && enabled && listener.available() && !registered(it.url,it.me)}
        val names=wanted.map {"compose-reconcile-${it.key}"}.toSet()
        val known=preferences.getStringSet("work-names",emptySet()).orEmpty()
        val work=WorkManager.getInstance(context)
        (known-names).forEach(work::cancelUniqueWork)
        // Persist before enqueue so process death cannot leave an untracked job.
        preferences.edit().putStringSet("work-names",known+names).commit()
        wanted.forEach {server ->work.enqueueUniquePeriodicWork("compose-reconcile-${server.key}",ExistingPeriodicWorkPolicy.UPDATE,
            PeriodicWorkRequest.Builder(MessageReconcileWorker::class.java,15,TimeUnit.MINUTES)
                .setInputData(workDataOf("server" to server.url,"owner" to server.me))
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL,15,TimeUnit.MINUTES).addTag("compose-reconcile").build())}
    }
}

class PapoMessagingService:FirebaseMessagingService() {
    override fun onNewToken(token:String) {MessageAlerts.start(this);MessageAlerts.newToken(token)}
    override fun onMessageReceived(message:RemoteMessage) {
        // Foreground delivery is handled by the core live connection. A push
        // wake can request a refresh without creating a second delivery path.
        MessageAlerts.start(this);MessageAlerts.pushHints.tryEmit(message.data["message_id"].orEmpty());MessageAlerts.refresh()
    }
    override fun onDeletedMessages() {MessageAlerts.start(this);MessageAlerts.pushHints.tryEmit("");MessageAlerts.refresh()}
}

/** Bounded REST fallback, not a permanent service or a second live connection. */
class MessageReconcileWorker(context:Context,parameters:WorkerParameters):Worker(context,parameters) {
    @Volatile private var reconcile:NotificationReconcile?=null
    override fun doWork():Result {
        MessageAlerts.start(applicationContext)
        val server=inputData.getString("server") ?: return Result.failure()
        val owner=inputData.getString("owner") ?: return Result.failure()
        if(MessageAlerts.owner(server)!=owner || !MessageAlerts.listener.available() || MessageAlerts.registered(server,owner)) return Result.success()
        val saved=org.json.JSONArray(applicationContext.getSharedPreferences("compose-client",0).getString("servers","[]"))
        if((0 until saved.length()).none {saved.optString(it)==server}) return Result.success()
        val task=NotificationReconcile();reconcile=task
        return try {
            if(isStopped) task.cancel()
            if(task.run(server,messageDatabasePath(applicationContext),AndroidSecrets(applicationContext),MessageAlerts.listener)) Result.retry() else Result.success()
        } catch(_:Exception) {Result.retry()} finally {reconcile=null;task.destroy()}
    }
    override fun onStopped() {reconcile?.cancel();super.onStopped()}
}
