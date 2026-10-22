@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.chriexpe.papo.compose

import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import dev.chrisbanes.haze.hazeSource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun FileSettings(feedback: (String) -> Unit) {
    val context = LocalContext.current
    val prefs = LocalClientPreferences.current
    val folder = prefs.string("download-tree")
    val scope = rememberCoroutineScope()
    val picker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
            if (uri != null)
                scope.launch {
                    try {
                        val label =
                            withContext(Dispatchers.IO) {
                                context.contentResolver.takePersistableUriPermission(
                                    uri,
                                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                                )
                                val document =
                                    DocumentsContract.buildDocumentUriUsingTree(
                                        uri,
                                        DocumentsContract.getTreeDocumentId(uri),
                                    )
                                context.contentResolver
                                    .query(
                                        document,
                                        arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),
                                        null,
                                        null,
                                        null,
                                    )
                                    ?.use { cursor ->
                                        if (cursor.moveToFirst()) cursor.getString(0) else null
                                    } ?: ui(R.string.selected_folder)
                            }
                        val previous = folder.takeIf { it.isNotBlank() }?.let(Uri::parse)
                        prefs.set("download-tree", uri.toString())
                        prefs.set("download-label", label)
                        if (previous != null && previous != uri)
                            withContext(Dispatchers.IO) {
                                runCatching {
                                    context.contentResolver.releasePersistableUriPermission(
                                        previous,
                                        Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                            Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                                    )
                                }
                            }
                        feedback(ui(R.string.download_folder_updated))
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        feedback(e.message ?: ui(R.string.couldn_t_use_that_folder))
                    }
                }
        }
    LazyColumn(
        Modifier.fillMaxSize().hazeSource(LocalHaze.current),
        contentPadding = PaddingValues(20.dp, 8.dp, 20.dp, 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item { SettingsLabel(ui(R.string.downloads)) }
        item {
            SettingsCard {
                SettingsLink(
                    ui(R.string.save_media_to),
                    downloadLocation(context),
                    Icons.Outlined.Folder,
                    { picker.launch(folder.takeIf { it.isNotBlank() }?.let(Uri::parse)) },
                    target = "download-folder",
                )
                if (folder.isNotBlank()) {
                    SettingsDivider()
                    SettingsLink(
                        ui(R.string.use_downloads_papo),
                        ui(R.string.return_to_the_default_android_folder),
                        Icons.Outlined.Restore,
                        {
                            val old = Uri.parse(folder)
                            prefs.set("download-tree", "")
                            prefs.set("download-label", "")
                            runCatching {
                                context.contentResolver.releasePersistableUriPermission(
                                    old,
                                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
                                )
                            }
                            feedback(ui(R.string.downloads_will_be_saved_to_downloads_papo))
                        },
                        target = "download-reset",
                    )
                }
            }
        }
        item {
            Text(
                ui(R.string.download_folder_description),
                Modifier.padding(horizontal = 8.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
