@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.SystemUpdateAlt
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import java.io.File
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject

private const val RELEASE_API = "https://api.github.com/repos/chriexpe/papo-frontend/releases/latest"
private const val CHECK_INTERVAL = 6 * 60 * 60 * 1000L
private const val ERROR_RETRY_INTERVAL = 15 * 60 * 1000L

internal data class UpdateRelease(
    val version: String,
    val notes: String,
    val apkUrl: String,
    val checksumUrl: String,
)

internal data class UpdateStatus(
    val release: UpdateRelease? = null,
    val downloading: Boolean = false,
    val downloaded: Long = 0,
    val total: Long? = null,
    val ready: Boolean = false,
    val error: String? = null,
)

/** Per-process Android handoff. The network and disk work never runs on the Compose thread. */
internal class AppUpdates(private val app: Application) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val preferences = app.getSharedPreferences("papo-updates", Context.MODE_PRIVATE)
    private val mutableStatus = MutableStateFlow(UpdateStatus())
    val status = mutableStatus.asStateFlow()
    var sheetOpen by mutableStateOf(false)
    private var operation: Job? = null
    private var nextCheckAt = 0L

    // Google Play is the update authority for Play installs. Development builds have
    // different application IDs/signatures and must never offer a production APK.
    val enabled: Boolean =
        !BuildConfig.DEBUG && run {
            try {
                val installer =
                    app.packageManager.getInstallSourceInfo(app.packageName).installingPackageName
                installer != "com.android.vending"
            } catch (_: Exception) {
                false
            }
        }

    fun check() {
        if (!enabled || operation?.isActive == true || System.currentTimeMillis() < nextCheckAt)
            return
        nextCheckAt = System.currentTimeMillis() + ERROR_RETRY_INTERVAL
        operation =
            scope.launch {
                try {
                    val release = withContext(Dispatchers.IO) { latestRelease() }
                    val ignored = preferences.getString("ignoredVersion", null)
                    val ready =
                        if (release != null && release.version != ignored)
                            withContext(Dispatchers.IO) { verifyCachedApk(release) }
                        else false
                    mutableStatus.value =
                        if (release == null || release.version == ignored) UpdateStatus()
                        else UpdateStatus(release = release, ready = ready)
                    nextCheckAt = System.currentTimeMillis() + CHECK_INTERVAL
                } catch (cancel: CancellationException) {
                    throw cancel
                } catch (_: Exception) {
                    // A failed background check must not erase an already discovered update.
                }
            }
    }

    fun download() {
        val release = status.value.release ?: return
        if (!enabled || operation?.isActive == true || status.value.ready) return
        mutableStatus.update { it.copy(downloading = true, downloaded = 0, total = null, error = null) }
        operation =
            scope.launch {
                try {
                    withContext(Dispatchers.IO) {
                        downloadApk(release) { bytes, total ->
                            mutableStatus.update {
                                if (it.release?.version == release.version)
                                    it.copy(downloaded = bytes, total = total)
                                else it
                            }
                        }
                    }
                    mutableStatus.update {
                        if (it.release?.version == release.version)
                            it.copy(downloading = false, ready = true, error = null)
                        else it
                    }
                } catch (cancel: CancellationException) {
                    throw cancel
                } catch (error: Exception) {
                    mutableStatus.update {
                        it.copy(downloading = false, error = error.message ?: "Download failed")
                    }
                }
            }
    }

    fun install(activity: Activity) {
        val release = status.value.release ?: return
        if (!enabled || !status.value.ready || operation?.isActive == true) return
        if (Build.VERSION.SDK_INT >= 26 && !activity.packageManager.canRequestPackageInstalls()) {
            activity.startActivity(
                Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${activity.packageName}"),
                )
            )
            return
        }
        operation =
            scope.launch {
                try {
                    // Verify again before the installer handoff, including after a permission trip.
                    val verified = withContext(Dispatchers.IO) { verifyCachedApk(release) }
                    if (!verified) {
                        mutableStatus.update { it.copy(ready = false, error = "APK verification failed") }
                        return@launch
                    }
                    val apk = apkFile(release)
                    val uri =
                        FileProvider.getUriForFile(activity, "${activity.packageName}.files", apk)
                    activity.startActivity(
                        Intent(Intent.ACTION_VIEW)
                            .setDataAndType(uri, "application/vnd.android.package-archive")
                            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    )
                } catch (cancel: CancellationException) {
                    throw cancel
                } catch (error: Exception) {
                    mutableStatus.update { it.copy(error = error.message ?: "Installer unavailable") }
                }
            }
    }

    fun ignoreVersion() {
        val release = status.value.release ?: return
        if (status.value.downloading) return
        preferences.edit().putString("ignoredVersion", release.version).apply()
        mutableStatus.value = UpdateStatus()
        sheetOpen = false
    }

    private fun apkFile(release: UpdateRelease): File =
        File(File(app.cacheDir, "updates"), "Papo-${release.version}-android-arm64.apk")

    private fun latestRelease(): UpdateRelease? {
        val json = JSONObject(requestText(RELEASE_API))
        val version = json.getString("tag_name").removePrefix("v")
        if (!newerStableRelease(version, BuildConfig.VERSION_NAME)) return null
        val filename = "Papo-$version-android-arm64.apk"
        val assets = json.getJSONArray("assets")
        var apk: String? = null
        var checksum: String? = null
        for (index in 0 until assets.length()) {
            val asset = assets.getJSONObject(index)
            when (asset.getString("name")) {
                filename -> apk = asset.getString("browser_download_url")
                "$filename.sha256" -> checksum = asset.getString("browser_download_url")
            }
        }
        // Never offer an APK without its matching release checksum.
        if (apk == null || checksum == null) return null
        requireReleaseUrl(apk)
        requireReleaseUrl(checksum)
        return UpdateRelease(version, json.optString("body"), apk, checksum)
    }

    private fun requireReleaseUrl(url: String) {
        val parsed = URL(url)
        require(
            parsed.protocol == "https" &&
                parsed.host == "github.com" &&
                parsed.path.startsWith("/chriexpe/papo-frontend/releases/download/")
        ) { "Unexpected release asset URL" }
    }

    private fun verifiedChecksum(release: UpdateRelease): String {
        val text = requestText(release.checksumUrl, 8192)
        val first = text.trim().split(Regex("\\s+")).firstOrNull().orEmpty().lowercase()
        require(Regex("[0-9a-f]{64}").matches(first)) { "Invalid release checksum" }
        return first
    }

    private fun verifyCachedApk(release: UpdateRelease): Boolean {
        val file = apkFile(release)
        if (!file.isFile) return false
        val matches = checksum(file) == verifiedChecksum(release)
        if (!matches) file.delete()
        return matches
    }

    private fun downloadApk(release: UpdateRelease, progress: (Long, Long?) -> Unit) {
        val dir = File(app.cacheDir, "updates")
        check(dir.isDirectory || dir.mkdirs()) { "Cannot create update cache" }
        val dest = apkFile(release)
        val temporary = File(dir, "${dest.name}.part")
        temporary.delete()
        try {
            request(release.apkUrl) { connection ->
                val total = connection.contentLengthLong.takeIf { it > 0 }
                var received = 0L
                connection.inputStream.use { stream ->
                    FileOutputStream(temporary).use { output ->
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            if (Thread.currentThread().isInterrupted) error("Download interrupted")
                            val length = stream.read(buffer)
                            if (length < 0) break
                            output.write(buffer, 0, length)
                            received += length
                            progress(received, total)
                        }
                        output.fd.sync()
                    }
                }
            }
            check(checksum(temporary) == verifiedChecksum(release)) {
                "Downloaded APK failed SHA-256 verification"
            }
            if (dest.exists()) check(dest.delete()) { "Cannot replace cached APK" }
            check(temporary.renameTo(dest)) { "Cannot save verified APK" }
        } finally {
            temporary.delete()
        }
    }

    private fun checksum(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { input ->
            val bytes = ByteArray(64 * 1024)
            while (true) {
                val size = input.read(bytes)
                if (size < 0) break
                digest.update(bytes, 0, size)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun requestText(url: String, limit: Int = 512 * 1024): String =
        request(url) { connection ->
            connection.inputStream.use { stream ->
                val buffer = ByteArray(8192)
                val output = java.io.ByteArrayOutputStream()
                while (true) {
                    val size = stream.read(buffer)
                    if (size < 0) break
                    check(output.size() + size <= limit) { "Release response too large" }
                    output.write(buffer, 0, size)
                }
                output.toString(Charsets.UTF_8.name())
            }
        }

    private inline fun <T> request(url: String, block: (HttpURLConnection) -> T): T {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.connectTimeout = 15_000
        connection.readTimeout = 30_000
        connection.setRequestProperty("User-Agent", "Papo/${BuildConfig.VERSION_NAME}")
        try {
            check(connection.responseCode in 200..299) {
                "Update request failed (HTTP ${connection.responseCode})"
            }
            return block(connection)
        } finally {
            connection.disconnect()
        }
    }
}

/** GitHub's latest release is stable; pre-release local builds are lower than a stable match. */
internal fun newerStableRelease(latest: String, installed: String): Boolean {
    val pattern =
        Regex("""^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$""")
    val a = pattern.matchEntire(latest) ?: return false
    val b = pattern.matchEntire(installed) ?: return false
    for (part in 1..3) {
        val lhs = a.groupValues[part].toLongOrNull() ?: return false
        val rhs = b.groupValues[part].toLongOrNull() ?: return false
        if (lhs != rhs) return lhs > rhs
    }
    return a.groupValues[4].isEmpty() && b.groupValues[4].isNotEmpty()
}

internal val LocalAppUpdates = staticCompositionLocalOf<AppUpdates?> { null }

/** Leaves the original header layout untouched when there is no update. */
@Composable
internal fun UpdateIndicator(modifier: Modifier = Modifier, fallback: @Composable () -> Unit = {}) {
    val updates = LocalAppUpdates.current
    if (updates == null) {
        fallback()
        return
    }
    val status by updates.status.collectAsStateWithLifecycle()
    if (status.release == null) {
        fallback()
        return
    }
    IconButton(
        onClick = { updates.sheetOpen = true },
        modifier = modifier.testTag("available-update"),
    ) {
        if (status.downloading)
            CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
        else
            Icon(
                if (status.ready) Icons.Outlined.SystemUpdateAlt else Icons.Outlined.Download,
                ui(R.string.update_available),
                tint = MaterialTheme.colorScheme.primary,
            )
    }
}

@Composable
internal fun UpdateSheet(updates: AppUpdates) {
    val status by updates.status.collectAsStateWithLifecycle()
    val release = status.release ?: return
    if (!updates.sheetOpen) return
    val activity = LocalContext.current as? Activity
    val height = LocalConfiguration.current.screenHeightDp.dp * 0.8f
    ModalBottomSheet(
        onDismissRequest = { updates.sheetOpen = false },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            Modifier.fillMaxWidth()
                .heightIn(max = height)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 24.dp)
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(ui(R.string.update_available), style = MaterialTheme.typography.titleLarge)
            Text(
                "Papo v${release.version}",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                release.notes.ifBlank { ui(R.string.update_no_notes) },
                style = MaterialTheme.typography.bodyMedium,
            )
            if (status.downloading) {
                val total = status.total
                if (total != null && total > 0)
                    LinearProgressIndicator(
                        progress = { (status.downloaded.toFloat() / total).coerceIn(0f, 1f) },
                        modifier = Modifier.fillMaxWidth(),
                    )
                else LinearProgressIndicator(Modifier.fillMaxWidth())
                Text(ui(R.string.update_downloading))
            }
            if (status.ready) Text(ui(R.string.update_ready))
            status.error?.let {
                Text(it, color = MaterialTheme.colorScheme.error)
            }
            Button(
                onClick = { if (status.ready) activity?.let(updates::install) else updates.download() },
                enabled = !status.downloading && activity != null,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("update-primary"),
            ) {
                Icon(
                    if (status.ready) Icons.Outlined.SystemUpdateAlt else Icons.Outlined.Download,
                    null,
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    if (status.ready) ui(R.string.update_install)
                    else ui(R.string.update_download)
                )
            }
            if (status.ready &&
                activity != null &&
                Build.VERSION.SDK_INT >= 26 &&
                !activity.packageManager.canRequestPackageInstalls()
            ) {
                Text(
                    ui(R.string.update_permission),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            TextButton(
                onClick = updates::ignoreVersion,
                enabled = !status.downloading,
                modifier = Modifier.align(Alignment.CenterHorizontally).testTag("ignore-update"),
            ) {
                Text(ui(R.string.update_ignore_version))
            }
            Spacer(Modifier.height(12.dp))
        }
    }
}
