package io.github.chriexpe.papo.compose

import android.app.NotificationManager
import android.content.Intent
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.MessageAlert
import io.github.chriexpe.papo.notifications.MessageNotifications
import org.junit.Assert.*
import org.junit.Test

class NotificationTest {
    private fun await(check:()->Boolean) {
        val until=android.os.SystemClock.elapsedRealtime()+5000
        while(!check() && android.os.SystemClock.elapsedRealtime()<until) android.os.SystemClock.sleep(20)
        assertTrue(check())
    }
    @Test fun rendererHonorsPermissionAndOwnerAndKeepsMessageIdentityStable() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val context=instrumentation.targetContext
        MessageAlerts.start(context)
        val manager=context.getSystemService(NotificationManager::class.java)
        val server="https://notification-fixture.invalid"
        val permission=android.Manifest.permission.POST_NOTIFICATIONS
        val prefs=context.getSharedPreferences("compose-appearance",0)
        val previous=prefs.getBoolean("notifications-enabled",true)
        prefs.edit().putBoolean("notifications-enabled",true).commit()
        instrumentation.uiAutomation.grantRuntimePermission(context.packageName,permission)
        try {
            MessageAlerts.setOwner(server,"current")
            val alert=MessageAlert(server,"earlier","chat","message","backend-notice","Title","Body")
            MessageAlerts.listener.show(alert)
            assertFalse(manager.activeNotifications.any {it.tag?.contains(server)==true})
            MessageAlerts.listener.show(alert.copy(ownerUserId="current"))
            MessageAlerts.listener.show(alert.copy(ownerUserId="current",notificationId="other-delivery"))
            await {manager.activeNotifications.count {it.tag?.contains(server)==true}==1}
            val notices=manager.activeNotifications.filter {it.tag?.contains(server)==true}
            assertEquals(1,notices.size)
            assertEquals(android.app.Notification.VISIBILITY_PRIVATE,notices.single().notification.visibility)
            MessageAlerts.clear(server,"earlier","chat")
            assertEquals(1,manager.activeNotifications.count {it.tag?.contains(server)==true})
            MessageAlerts.clear(server,"current","chat")
            await {!manager.activeNotifications.any {it.tag?.contains(server)==true}}
        } finally {
            MessageAlerts.clear(server,"current","chat")
            MessageAlerts.setOwner(server,"")
            prefs.edit().putBoolean("notifications-enabled",previous).commit()
        }
    }
    @Test fun nativeAndFirebaseTargetsAreConsumedAndMalformedIntentsAreIgnored() {
        val local=Intent().putExtra(MessageNotifications.EXTRA_SERVER,"https://papo.example").putExtra(MessageNotifications.EXTRA_OWNER,"owner")
            .putExtra(MessageNotifications.EXTRA_CHANNEL,"chat").putExtra(MessageNotifications.EXTRA_MESSAGE_ID,"message")
        assertEquals(NotificationTarget("https://papo.example","owner","chat","message"),MessageAlerts.target(local))
        assertNull(MessageAlerts.target(local))
        val push=Intent().putExtra("google.message_id","fcm").putExtra("channel_id","chat").putExtra("message_id","message")
        assertEquals(NotificationTarget("","","chat","message"),MessageAlerts.target(push))
        assertNull(MessageAlerts.target(push))
        assertNull(MessageAlerts.target(Intent().putExtra("channel_id","chat").putExtra("message_id","message")))
        assertNull(MessageAlerts.target(Intent().putExtra(MessageNotifications.EXTRA_MESSAGE_ID,"message")))
    }
}
