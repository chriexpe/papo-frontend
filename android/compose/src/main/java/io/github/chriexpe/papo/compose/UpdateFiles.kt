package io.github.chriexpe.papo.compose

import android.content.Context
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Both the UI's cached-APK check and WorkManager's downloader use the same file and checksum
 * contract. No APK becomes installable until its SHA-256 has been verified.
 */
internal object UpdateFiles {
    const val MAX_APK_BYTES = 512L * 1024 * 1024

    fun filename(version: String): String {
        require(Regex("""^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$""").matches(version))
        return "Papo-$version-android-arm64.apk"
    }

    fun apk(context: Context, release: UpdateRelease): File =
        File(File(context.cacheDir, "updates"), filename(release.version))

    fun partial(context: Context, release: UpdateRelease): File =
        File(apk(context, release).absolutePath + ".part")

    fun validator(context: Context, release: UpdateRelease): File =
        File(apk(context, release).absolutePath + ".etag")

    /** The release feed is trusted, but never follow arbitrary installer/checksum URLs. */
    fun validate(release: UpdateRelease) {
        val apkName = filename(release.version)
        requireAsset(release.apkUrl, apkName)
        requireAsset(release.checksumUrl, "$apkName.sha256")
    }

    private fun requireAsset(value: String, name: String) {
        val url = URL(value)
        require(
            url.protocol == "https" &&
                url.host == "github.com" &&
                url.port == -1 &&
                url.userInfo == null &&
                url.query == null &&
                url.ref == null &&
                url.path ==
                    "/chriexpe/papo-frontend/releases/download/v" +
                        name.removePrefix("Papo-").substringBefore("-android-arm64") +
                        "/$name"
        ) {
            "Unexpected update asset URL"
        }
    }

    fun checksum(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    fun expectedChecksum(release: UpdateRelease): String {
        validate(release)
        val checksum =
            requestText(release.checksumUrl, 8192)
                .trim()
                .split(Regex("\\s+"))
                .firstOrNull()
                .orEmpty()
                .lowercase()
        require(Regex("[a-f0-9]{64}").matches(checksum)) { "Invalid update checksum" }
        return checksum
    }

    fun cachedApkVerified(context: Context, release: UpdateRelease): Boolean {
        val file = apk(context, release)
        if (!file.isFile) return false
        val valid = checksum(file) == expectedChecksum(release)
        if (!valid) file.delete()
        return valid
    }

    fun requestText(url: String, limit: Int = 512 * 1024): String =
        request(url) { connection ->
            connection.inputStream.use { stream ->
                val buffer = ByteArray(8192)
                val output = java.io.ByteArrayOutputStream()
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    check(output.size() + count <= limit) { "Update response too large" }
                    output.write(buffer, 0, count)
                }
                output.toString(Charsets.UTF_8.name())
            }
        }

    inline fun <T> request(url: String, block: (HttpURLConnection) -> T): T {
        val connection = open(url)
        try {
            val status = connection.responseCode
            check(status in 200..299) { "Update request failed (HTTP $status)" }
            return block(connection)
        } finally {
            connection.disconnect()
        }
    }

    fun open(url: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            setRequestProperty("User-Agent", "Papo/${BuildConfig.VERSION_NAME}")
            setRequestProperty("Accept-Encoding", "identity")
        }
}
