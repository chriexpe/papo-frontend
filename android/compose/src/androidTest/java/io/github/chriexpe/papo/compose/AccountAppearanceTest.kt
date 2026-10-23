package io.github.chriexpe.papo.compose

import android.content.ContextWrapper
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.AppearanceChange
import io.github.chriexpe.papo.core.AppearanceException
import io.github.chriexpe.papo.core.AppearanceView
import io.github.chriexpe.papo.core.NativeClient
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class AccountAppearanceTest {
    @Test
    fun migration_ack_cache_retry_and_late_results_remain_per_account() {
        val base = InstrumentationRegistry.getInstrumentation().targetContext
        val name = "appearance-fixture-${System.nanoTime()}"
        val context =
            object : ContextWrapper(base) {
                override fun getSharedPreferences(requested: String, mode: Int) =
                    super.getSharedPreferences(
                        if (requested == "compose-appearance") name else requested,
                        mode,
                    )
            }
        val storage = context.getSharedPreferences("compose-appearance", 0)
        storage.edit().putBoolean("compact", true).commit()
        val client = NativeClient.demo()
        val fixture =
            try {
                client.poll()
            } finally {
                client.close()
            }
        val a = fixture.copy(url = "https://a.test", me = "account-a", appearance = null)
        val b =
            fixture.copy(
                url = "https://b.test",
                me = "account-b",
                appearance = AppearanceView("medium", "comfortable"),
            )
        val active = mutableListOf(a, b)
        val requests = mutableListOf<Pair<AppearanceOwner, AppearanceChange?>>()
        val waiting = ArrayDeque<CompletableDeferred<AppearanceView>>()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        fun cached(server: ServerState): MessageAppearance {
            val preferences = ClientPreferences(storage)
            return try {
                preferences.messageAppearance(server.copy(appearance = null))
            } finally {
                preferences.close()
            }
        }
        val coordinator =
            AccountAppearanceCoordinator(
                context,
                scope,
                { server -> active.any { it.url == server.url && it.me == server.me } },
                { server, change ->
                    requests += server.appearanceOwner() to change
                    withContext(NonCancellable) {
                        CompletableDeferred<AppearanceView>().also(waiting::addLast).await()
                    }
                },
            )
        try {
            coordinator.refresh(a)
            assertTrue(requests.last().second is AppearanceChange.ImportLegacyCompact)
            assertFalse(storage.getBoolean(a.appearanceOwner().cachePrefix() + "migrated", false))
            waiting.removeFirst().complete(AppearanceView("huge", "compact"))
            assertEquals(AccountFontSize.Large, cached(a).font)
            assertEquals(MessageDensity.Compact, cached(a).density)
            assertTrue(storage.getBoolean(a.appearanceOwner().cachePrefix() + "migrated", false))

            coordinator.change(a, AppearanceChange.FontSize("small"))
            waiting.removeFirst().completeExceptionally(AppearanceException.Network())
            assertEquals(AccountFontSize.Large, cached(a).font)
            assertEquals(
                R.string.appearance_network_error,
                coordinator.states[a.appearanceOwner()]?.error,
            )
            coordinator.retry(a)
            assertEquals("small", (requests.last().second as AppearanceChange.FontSize).value)
            waiting.removeFirst().complete(AppearanceView("small", "compact"))
            assertEquals(AccountFontSize.Small, cached(a).font)

            coordinator.refresh(b)
            assertTrue(requests.last().second is AppearanceChange.ImportLegacyCompact)
            waiting.removeFirst().complete(AppearanceView("medium", "comfortable"))
            assertEquals(MessageDensity.Comfortable, cached(b).density)
            assertEquals(MessageDensity.Compact, cached(a).density)

            coordinator.change(b, AppearanceChange.MessageDensity("normal"))
            val late = waiting.removeFirst()
            active.remove(b)
            coordinator.synchronize(active)
            active.add(b)
            coordinator.refresh(b, force = true)
            val newer = waiting.removeFirst()
            late.complete(AppearanceView("huge", "normal"))
            assertTrue(coordinator.states[b.appearanceOwner()]?.busy == true)
            assertEquals(MessageDensity.Comfortable, cached(b).density)
            newer.complete(AppearanceView("small", "comfortable"))
            assertEquals(AccountFontSize.Small, cached(b).font)
            coordinator.forget(a.url)
            assertFalse(storage.all.keys.any { it.startsWith(appearanceServerPrefix(a.url)) })
            assertTrue(storage.all.keys.any { it.startsWith(appearanceServerPrefix(b.url)) })
        } finally {
            scope.cancel()
            base.deleteSharedPreferences(name)
        }
    }

    @Test
    fun typography_and_spacing_keep_the_accepted_baseline_and_accessible_labels() {
        assertEquals(14f, messageTypography(AccountFontSize.Small).bodyLarge.fontSize.value, .001f)
        assertEquals(16f, messageTypography(AccountFontSize.Normal).bodyLarge.fontSize.value, .001f)
        assertEquals(20f, messageTypography(AccountFontSize.Large).bodyLarge.fontSize.value, .001f)
        assertTrue(messageTypography(AccountFontSize.Small).labelSmall.fontSize.value >= 11f)
        val choices = MessageDensity.entries.map { MessageAppearance(density = it) }
        assertTrue(choices[0].lineHeight < choices[1].lineHeight)
        assertTrue(choices[1].lineHeight < choices[2].lineHeight)
        assertTrue(choices[0].continuationGap < choices[1].continuationGap)
        assertTrue(choices[1].continuationGap < choices[2].continuationGap)
    }
}
