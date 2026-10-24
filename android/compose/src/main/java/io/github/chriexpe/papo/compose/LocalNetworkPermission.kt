package io.github.chriexpe.papo.compose

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import java.net.URI

/** SDK 37 protects LAN servers too; public-server connections do not need this permission. */
internal fun localServer(url: String): Boolean {
    if (Build.VERSION.SDK_INT < 37) return false
    val host =
        runCatching { URI(url).host?.lowercase()?.removeSurrounding("[", "]") }.getOrNull()
            ?: return false
    val first = host.substringBefore('.').toIntOrNull()
    val second = host.substringAfter('.').substringBefore('.').toIntOrNull()
    return host == "localhost" ||
        host.endsWith(".local") ||
        host.endsWith(".lan") ||
        first == 10 ||
        first == 127 ||
        (first == 192 && second == 168) ||
        (first == 172 && second in 16..31) ||
        (first == 169 && second == 254) ||
        host == "::1" ||
        host.startsWith("fc") && host.contains(':') ||
        host.startsWith("fd") && host.contains(':') ||
        host.startsWith("fe80:")
}

@Composable
internal fun LocalNetworkPermission(model: PapoViewModel, servers: List<ServerState>) {
    var asked by rememberSaveable { mutableStateOf(false) }
    val launcher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            if (allowed) model.resume()
            else model.voiceCalls.reportError("local-network-permission")
        }
    val required = servers.any { localServer(it.url) }
    LaunchedEffect(required) {
        if (
            required &&
                !asked &&
                !AndroidCallRuntime.granted(Manifest.permission.ACCESS_LOCAL_NETWORK)
        ) {
            asked = true
            launcher.launch(Manifest.permission.ACCESS_LOCAL_NETWORK)
        }
    }
}
