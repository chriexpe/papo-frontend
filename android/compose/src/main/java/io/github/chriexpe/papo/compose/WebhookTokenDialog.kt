package io.github.chriexpe.papo.compose

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import io.github.chriexpe.papo.core.WebhookReceipt

@Composable
internal fun WebhookTokenDialog(
    server: ServerState,
    receipt: WebhookReceipt,
    onDismiss: () -> Unit,
) {
    // Never use rememberSaveable or a ViewModel for credentials. Rotation,
    // backgrounding and leaving management discard this one-time disclosure.
    val token = remember(receipt) { receipt.takeToken() }
    val webhook = remember(receipt) { receipt.webhook() }
    var copied by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val dismiss by rememberUpdatedState(onDismiss)
    DisposableEffect(lifecycle, receipt) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) dismiss()
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
        modifier = Modifier.testTag("webhook-token-dialog"),
        title = { Text(ui(R.string.webhook_token_title, webhook.name)) },
        text = {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(ui(R.string.webhook_token_warning))
                // Plain Text avoids autofill, saved editor state and selection's
                // automatic clipboard path; copying below marks it sensitive.
                Text(token, fontFamily = FontFamily.Monospace)
                FilledTonalButton(
                    enabled = token.isNotBlank(),
                    modifier = Modifier.testTag("copy-webhook-token"),
                    onClick = {
                        val clip = ClipData.newPlainText(ui(R.string.webhook_token), token)
                        clip.description.extras =
                            PersistableBundle().apply {
                                putBoolean("android.content.extra.IS_SENSITIVE", true)
                            }
                        (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
                            .setPrimaryClip(clip)
                        copied = true
                    },
                ) {
                    Text(ui(if (copied) R.string.copied else R.string.copy_token))
                }
                Text(
                    ui(R.string.webhook_execution_endpoint),
                    style = MaterialTheme.typography.labelLarge,
                )
                Text(
                    java.net.URI(server.url).resolve("/webhooks/${webhook.id}/messages").toString(),
                    fontFamily = FontFamily.Monospace,
                )
                Text(
                    ui(R.string.webhook_token_header_hint),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(ui(R.string.done)) } },
    )
}
