package io.github.chriexpe.papo.compose

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.io.File

/** Publish only complete files. Interrupted saves never leave broken downloads. */
internal suspend fun saveMediaDownload(context: Context, file: File, name: String, mime: String): Uri = withContext(Dispatchers.IO) {
    val resolver=context.contentResolver
    val values=ContentValues().apply {
        put(MediaStore.MediaColumns.DISPLAY_NAME,name.replace(Regex("[/\\\\\\p{Cntrl}]"),"_").take(180).ifBlank {"Papo media"})
        put(MediaStore.MediaColumns.MIME_TYPE,mime)
        put(MediaStore.MediaColumns.RELATIVE_PATH,"${Environment.DIRECTORY_DOWNLOADS}/Papo")
        put(MediaStore.MediaColumns.IS_PENDING,1)
    }
    val uri=resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI,values) ?: error("Couldn’t create the download")
    try {
        val work=kotlinx.coroutines.currentCoroutineContext()
        requireNotNull(resolver.openOutputStream(uri)).use {output -> file.inputStream().use {input ->
            val buffer=ByteArray(64*1024)
            while(true) {work.ensureActive();val count=input.read(buffer);if(count<0) break;output.write(buffer,0,count)}
        } }
        work.ensureActive()
        check(resolver.update(uri,ContentValues().apply {put(MediaStore.MediaColumns.IS_PENDING,0)},null,null)==1) {"Couldn’t finish the download"}
        uri
    } catch(e:Exception) {resolver.delete(uri,null,null);throw e}
}
