package io.github.chriexpe.papo.compose

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ForegroundInfo
import androidx.work.WorkerParameters
import java.io.FileOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.currentCoroutineContext

/**
 * A unique, persistent WorkManager job. The .part and .etag files survive process death.
 * Range requests resume only when the server confirms the exact byte offset and validator.
 */
internal class UpdateDownloadWorker(context: Context, params: WorkerParameters) :
    CoroutineWorker(context, params) {
    companion object {
        const val UNIQUE_NAME = "papo-apk-update"
        const val VERSION = "version"
        const val APK_URL = "apkUrl"
        const val CHECKSUM_URL = "checksumUrl"
        const val DOWNLOADED = "downloaded"
        const val TOTAL = "total"
        const val ERROR = "error"
        private const val CHANNEL = "app-updates"
        private const val NOTIFICATION_ID = 2509
    }

    override suspend fun doWork(): Result {
        val release =
            try {
                UpdateRelease(
                    inputData.getString(VERSION).orEmpty(),
                    "",
                    inputData.getString(APK_URL).orEmpty(),
                    inputData.getString(CHECKSUM_URL).orEmpty(),
                ).also(UpdateFiles::validate)
            } catch (_: Exception) {
                return Result.failure(Data.Builder().putString(ERROR, "Invalid update metadata").build())
            }
        return try {
            setForeground(foreground(0, null))
            val expected = UpdateFiles.expectedChecksum(release)
            val target = UpdateFiles.apk(applicationContext, release)
            if (target.isFile && UpdateFiles.checksum(target) == expected) {
                setProgress(progress(target.length(), target.length()))
                return Result.success()
            }
            if (target.exists()) target.delete()
            download(release, expected)
            Result.success()
        } catch (cancel: CancellationException) {
            throw cancel
        } catch (error: Exception) {
            if (runAttemptCount < 4 && error is IOException) Result.retry()
            else Result.failure(Data.Builder().putString(ERROR, error.message ?: "Download failed").build())
        }
    }

    private suspend fun download(release: UpdateRelease, expected: String) {
        val target = UpdateFiles.apk(applicationContext, release)
        val partial = UpdateFiles.partial(applicationContext, release)
        val etagFile = UpdateFiles.validator(applicationContext, release)
        check(target.parentFile?.isDirectory == true || target.parentFile?.mkdirs() == true) {
            "Cannot create update cache"
        }
        var existing = partial.takeIf { it.isFile }?.length() ?: 0L
        if (existing > UpdateFiles.MAX_APK_BYTES) {
            partial.delete()
            etagFile.delete()
            existing = 0L
        }
        val validator = etagFile.takeIf { it.isFile }?.readText()?.takeIf { it.isNotBlank() }
        // Without an HTTP entity validator, an old partial must not be appended to a new APK.
        if (existing > 0 && validator == null) {
            partial.delete()
            existing = 0
        }

        var connection = UpdateFiles.open(release.apkUrl)
        try {
            if (existing > 0 && validator != null) {
                connection.setRequestProperty("Range", "bytes=$existing-")
                connection.setRequestProperty("If-Range", validator)
            }
            var code = connection.responseCode
            if (code == 416 || (code == 206 && !validRange(connection, existing))) {
                connection.disconnect()
                partial.delete()
                etagFile.delete()
                existing = 0
                connection = UpdateFiles.open(release.apkUrl)
                code = connection.responseCode
            }
            check(code == 200 || (code == 206 && existing > 0 && validRange(connection, existing))) {
                "Unexpected update response (HTTP $code)"
            }
            val append = code == 206 && existing > 0
            if (!append) {
                existing = 0
                partial.delete()
                etagFile.delete()
            }
            val newValidator =
                connection.getHeaderField("ETag")?.takeIf {
                    it.isNotBlank() && !it.startsWith("W/") && !it.contains('\n')
                }
            // If a 206 response has a different ETag, never mix the two payloads.
            if (append && validator != null && newValidator != validator) {
                throw IOException("Update changed during resume")
            }
            if (newValidator != null) etagFile.writeText(newValidator)
            val remaining = connection.contentLengthLong.takeIf { it >= 0 }
            val total = remaining?.plus(existing)
            check(total == null || total <= UpdateFiles.MAX_APK_BYTES) { "Update APK too large" }
            var received = existing
            var lastReport = 0L
            connection.inputStream.use { stream ->
                FileOutputStream(partial, append).use { output ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        currentCoroutineContext().ensureActive()
                        if (isStopped) throw CancellationException("Update stopped")
                        val count = stream.read(buffer)
                        if (count < 0) break
                        received += count
                        check(received <= UpdateFiles.MAX_APK_BYTES) { "Update APK too large" }
                        output.write(buffer, 0, count)
                        val now = android.os.SystemClock.elapsedRealtime()
                        if (now - lastReport >= 500) {
                            report(received, total)
                            lastReport = now
                        }
                    }
                    output.fd.sync()
                }
            }
            check(total == null || received == total) { "Incomplete update download" }
            currentCoroutineContext().ensureActive()
            check(UpdateFiles.checksum(partial) == expected) { "Update checksum mismatch" }
            if (target.exists()) check(target.delete()) { "Cannot replace cached APK" }
            check(partial.renameTo(target)) { "Cannot save downloaded APK" }
            etagFile.delete()
            report(received, received)
        } finally {
            connection.disconnect()
        }
    }

    private fun validRange(connection: HttpURLConnection, offset: Long): Boolean {
        val range = connection.getHeaderField("Content-Range") ?: return false
        val match = Regex("""bytes (\d+)-(\d+)/(\d+|\*)""").matchEntire(range) ?: return false
        val start = match.groupValues[1].toLongOrNull() ?: return false
        val end = match.groupValues[2].toLongOrNull() ?: return false
        return start == offset && end >= start
    }

    private suspend fun report(bytes: Long, total: Long?) {
        setProgress(progress(bytes, total))
        setForeground(foreground(bytes, total))
    }

    private fun progress(bytes: Long, total: Long?): Data =
        Data.Builder()
            .putLong(DOWNLOADED, bytes)
            .putLong(TOTAL, total ?: -1)
            .build()

    private fun foreground(bytes: Long, total: Long?): ForegroundInfo {
        val manager =
            applicationContext.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "App updates", NotificationManager.IMPORTANCE_LOW)
        )
        val intent = Intent(applicationContext, MainActivity::class.java)
        val pending =
            PendingIntent.getActivity(
                applicationContext,
                0,
                intent,
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        val percent =
            if (total != null && total > 0)
                ((bytes.toDouble() / total) * 100).toInt().coerceIn(0, 100)
            else 0
        val notification: Notification =
            NotificationCompat.Builder(applicationContext, CHANNEL)
                .setSmallIcon(R.drawable.ic_message_notification)
                .setContentTitle("Papo update")
                .setContentText(
                    if (total != null && total > 0) "Downloading: $percent%"
                    else "Downloading and verifying…"
                )
                .setContentIntent(pending)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setProgress(100, percent, total == null)
                .build()
        return if (Build.VERSION.SDK_INT >= 29)
            ForegroundInfo(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        else ForegroundInfo(NOTIFICATION_ID, notification)
    }
}
