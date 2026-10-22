package io.github.chriexpe.papo.compose

import android.content.res.Configuration
import android.os.LocaleList
import androidx.test.platform.app.InstrumentationRegistry
import io.github.chriexpe.papo.core.RecoveryException
import io.github.chriexpe.papo.core.recoverAccount
import io.github.chriexpe.papo.core.recoveryServer
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.net.ServerSocket
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class LinkRecoveryTest {
    private val context get()=InstrumentationRegistry.getInstrumentation().targetContext

    @Test fun trustedDomainsAreCanonicalExactHostsAndPersistRevocation() {
        val storage=context.getSharedPreferences("link-test-${System.nanoTime()}",0)
        val prefs=ClientPreferences(storage)
        try {
            assertTrue(prefs.boolean("confirm-external-links"))
            prefs.trust("HTTPS://Example.COM./")
            assertEquals(setOf("example.com"),prefs.trustedDomains)
            assertFalse("sub.example.com" in prefs.trustedDomains)
            assertFalse("example.com.evil.test" in prefs.trustedDomains)
            assertEquals("xn--bcher-kva.example",trustedDomain("bücher.example"))
            for(raw in listOf("https://a@evil.test","https://example.com/path","example.com?x=1","example.com#x","not a domain","javascript:alert(1)")) assertNull(raw,trustedDomain(raw))
            val reopened=ClientPreferences(storage)
            try {assertEquals(prefs.trustedDomains,reopened.trustedDomains);reopened.untrust("example.com");assertTrue(reopened.trustedDomains.isEmpty())} finally {reopened.close()}
        } finally {prefs.close();storage.edit().clear().commit()}
    }

    @Test fun englishAndPortugueseResourcesIncludePluralAndAccessibilityWording() {
        fun resources(tag:String)=context.createConfigurationContext(Configuration(context.resources.configuration).apply {setLocales(LocaleList.forLanguageTags(tag))}).resources
        val english=resources("en");val portuguese=resources("pt-BR")
        assertEquals("Search settings",english.getString(R.string.search_settings))
        assertEquals("Buscar nos ajustes",portuguese.getString(R.string.search_settings))
        assertEquals("Open external link?",english.getString(R.string.open_external_link))
        assertEquals("Abrir link externo?",portuguese.getString(R.string.open_external_link))
        assertEquals("Enviar mensagem",portuguese.getString(R.string.send_message))
        assertEquals("1 pessoa",portuguese.getQuantityString(R.plurals.people_count,1,1))
        assertEquals("2 pessoas",portuguese.getQuantityString(R.plurals.people_count,2,2))
        assertEquals("0 new messages",english.getQuantityString(R.plurals.new_message_count,0,0))
    }

    @Test fun recoveryRejectsForeignOriginsAndAmbiguousHintsBeforeHttp() {
        assertEquals("https://example.test",recoveryServer("papo-reset://passwordchange/token?server=https%3A%2F%2Fexample.test"))
        try {recoveryServer("https://evil.test/passwordchange/token");fail("Foreign recovery origin accepted")} catch(_:RecoveryException.InvalidLink) {}
        try {recoveryServer("papo-reset://passwordchange/token?server=https://one.test&server=https://two.test");fail("Ambiguous hints accepted")} catch(_:RecoveryException.InvalidLink) {}
        try {recoverAccount("http://127.0.0.1:9","papo-reset://passwordchange/token?server=https://example.test","NewPassword!");fail("Cross-server reset accepted")} catch(_:RecoveryException.WrongServer) {}
    }

    @Test fun recoveryUsesPublicCoreRouteAndNeverFollowsPasswordRedirects() {
        ServerSocket(0).use {server ->
            ServerSocket(0).use {redirect ->
                redirect.soTimeout=250
                val executor=Executors.newSingleThreadExecutor()
                val received=executor.submit<Pair<List<String>,JSONObject>> {
                    server.accept().use {socket ->
                        socket.soTimeout=5000
                        val input=socket.getInputStream().bufferedReader()
                        val headers=mutableListOf<String>()
                        while(true) {val line=input.readLine() ?: break;if(line.isEmpty()) break;headers.add(line)}
                        val size=headers.first {it.startsWith("content-length:",true)}.substringAfter(':').trim().toInt()
                        val body=CharArray(size);var offset=0
                        while(offset<size) {val count=input.read(body,offset,size-offset);check(count>0);offset+=count}
                        socket.getOutputStream().write(("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:${redirect.localPort}/capture\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").toByteArray())
                        headers to JSONObject(String(body))
                    }
                }
                try {
                    try {recoverAccount("http://127.0.0.1:${server.localPort}","papo-reset://passwordchange/one_time_token","NewPassword!");fail("Redirect must fail")} catch(_:RecoveryException) {}
                    val (headers,body)=received.get(10,TimeUnit.SECONDS)
                    assertEquals("POST /auth/password_reset HTTP/1.1",headers.first())
                    assertFalse(headers.any {it.startsWith("cookie:",true) || it.startsWith("authorization:",true)})
                    assertEquals("one_time_token",body.getString("token"));assertEquals("NewPassword!",body.getString("password"))
                    try {redirect.accept().use {};fail("Password followed redirect")} catch(_:java.net.SocketTimeoutException) {}
                } finally {executor.shutdownNow()}
            }
        }
    }
    @Test fun onlyAnAuthenticatedServerOwnerCanGenerateScopedRecoveryLinks() {
        for(owner in listOf(false,true)) IncomingChatFixture(context,ownerIsMe=owner).use {fixture ->
            val values=java.util.concurrent.ConcurrentHashMap<String,String>()
            val credentials=object:io.github.chriexpe.papo.core.Credentials {
                override fun load(server:String,secret:String)=values["$server/$secret"]
                override fun store(server:String,secret:String,value:String) {values["$server/$secret"]=value}
                override fun remove(server:String,secret:String) {values.remove("$server/$secret")}
            }
            val db=context.cacheDir.resolve("recovery-${System.nanoTime()}.db")
            val client=io.github.chriexpe.papo.core.NativeClient.open(fixture.url,db.absolutePath,credentials,CoreWake {})
            try {
                client.dispatch(io.github.chriexpe.papo.core.Action.Login("reader","FixturePassword!"))
                val deadline=android.os.SystemClock.elapsedRealtime()+15000
                var state=client.poll()
                while((state.me!="me" || state.members.size<2 || state.serverName!="Incoming fixture") && android.os.SystemClock.elapsedRealtime()<deadline) {Thread.sleep(20);state=client.poll()}
                assertEquals("me",state.me);assertEquals(owner,state.canResetPasswords)
                if(owner) {
                    val receipt=client.passwordResetLink(state.me,"other")
                    assertEquals(fixture.url,recoveryServer(receipt.url));assertTrue(receipt.expiresAt.isNotEmpty())
                    assertTrue(fixture.requests.contains("POST /users/other/reset"))
                    try {client.passwordResetLink(state.me,state.me);fail("Own reset uses a different backend contract")} catch(_:RecoveryException.InvalidMember) {}
                } else {
                    try {client.passwordResetLink(state.me,"other");fail("Non-owner reset accepted")} catch(_:RecoveryException.NotOwner) {}
                    assertFalse(fixture.requests.any {it.endsWith("/reset")})
                }
            } finally {client.shutdown();client.close();db.delete();context.cacheDir.resolve(db.name+"-wal").delete();context.cacheDir.resolve(db.name+"-shm").delete()}
        }
    }

}
