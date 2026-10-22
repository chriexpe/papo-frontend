package io.github.chriexpe.papo.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import java.net.IDN
import java.net.URI

/** Trust is an exact canonical hostname, never a suffix or a display label.
 * Unicode hosts are displayed as ASCII so the remembered identity is visible. */
internal fun externalLinkHost(raw:String):String? = runCatching {
    val uri=android.net.Uri.parse(raw)
    if(uri.scheme?.lowercase(java.util.Locale.ROOT) !in setOf("http","https") || uri.userInfo!=null) return null
    val host=uri.host?.trimEnd('.')?.takeIf {it.isNotBlank()} ?: return null
    if(host.contains(':')) return URI(raw).host?.lowercase(java.util.Locale.ROOT)
    IDN.toASCII(host,IDN.USE_STD3_ASCII_RULES).lowercase(java.util.Locale.ROOT)
}.getOrNull()

internal fun trustedDomain(raw:String):String? {
    val value=raw.trim()
    if(value.any {it.isWhitespace()} || value.contains('@')) return null
    val url=if(value.contains("://")) value else "https://$value"
    val parsed=android.net.Uri.parse(url)
    if(parsed.query!=null || parsed.fragment!=null || parsed.path !in listOf(null,"","/")) return null
    return externalLinkHost(url)
}

@Composable
internal fun ExternalLinkHost(onRecovery:(String)->Unit,content:@Composable ()->Unit) {
    val system=LocalUriHandler.current
    val prefs=LocalClientPreferences.current
    var pending by remember {mutableStateOf<String?>(null)}
    var rememberHost by remember {mutableStateOf(false)}
    var failed by remember {mutableStateOf(false)}
    fun launch(url:String) {failed=runCatching {system.openUri(url)}.isFailure}
    val handler=remember(system,prefs,onRecovery) {object:UriHandler {
        override fun openUri(uri:String) {
            if(isRecoveryIntent(uri)) {onRecovery(uri);return}
            val host=externalLinkHost(uri)
            val mail=runCatching {URI(uri).scheme.equals("mailto",true) && !uri.contains('\n') && !uri.contains('\r')}.getOrDefault(false)
            if(host==null && !mail) return
            if(!prefs.boolean("confirm-external-links") || host!=null && host in prefs.trustedDomains) launch(uri)
            else {rememberHost=false;pending=uri}
        }
    }}
    CompositionLocalProvider(LocalUriHandler provides handler,content=content)
    pending?.let {url ->
        val host=externalLinkHost(url)
        AlertDialog(onDismissRequest={pending=null},modifier=Modifier.testTag("external-link-confirmation"),
            icon={Icon(Icons.Outlined.OpenInNew,null)},title={Text(ui(R.string.open_external_link))},
            text={Column(Modifier.heightIn(max=320.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Text(ui(R.string.this_link_opens_outside_papo))
                if(host!=null) Text(host,style=MaterialTheme.typography.titleMedium)
                Text(url,style=MaterialTheme.typography.bodySmall)
                if(host!=null) Row(Modifier.clickable {rememberHost=!rememberHost},verticalAlignment=androidx.compose.ui.Alignment.CenterVertically) {
                    Checkbox(rememberHost,{rememberHost=it});Text(ui(R.string.always_trust_1_s, host))
                }
            }},confirmButton={TextButton(onClick={if(rememberHost && host!=null) prefs.trust(host);pending=null;launch(url)}) {Text(ui(R.string.open_link))}},
            dismissButton={TextButton(onClick={pending=null}) {Text(ui(R.string.cancel))}})
    }
    if(failed) AlertDialog(onDismissRequest={failed=false},title={Text(ui(R.string.couldn_t_open_link))},text={Text(ui(R.string.no_app_is_available_to_open_this_link))},confirmButton={TextButton(onClick={failed=false}) {Text(ui(R.string.ok))}})
}

@Composable
internal fun LinkSettings() {
    val prefs=LocalClientPreferences.current
    var domain by remember {mutableStateOf("")}
    val host=trustedDomain(domain)
    androidx.compose.foundation.lazy.LazyColumn(Modifier.fillMaxSize().testTag("link-settings"),contentPadding=PaddingValues(20.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        item {SettingsCard {SettingsSwitch(ui(R.string.confirm_external_links),ui(R.string.ask_before_opening_an_unfamiliar_website),prefs.boolean("confirm-external-links"),"confirm-external-links") {prefs.set("confirm-external-links",it)}}}
        item {SettingsLabel(ui(R.string.trusted_domains))}
        item {Text(ui(R.string.links_to_these_exact_domains_open_without_asking_subdomains_are_managed_sep))}
        item {SettingsCard {Column(Modifier.padding(16.dp).then(settingTarget("trusted-domains")),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(domain,{domain=it},Modifier.fillMaxWidth().testTag("trusted-domain-field"),label={Text(ui(R.string.domain))},placeholder={Text("example.com")},singleLine=true,isError=domain.isNotBlank() && host==null)
            Button(enabled=host!=null,onClick={prefs.trust(host!!);domain=""}) {Text(ui(R.string.add_trusted_domain))}
        }}}
        if(prefs.trustedDomains.isEmpty()) item {Text(ui(R.string.no_trusted_domains_yet),color=MaterialTheme.colorScheme.onSurfaceVariant)}
        prefs.trustedDomains.sorted().forEach {entry -> item(entry) {SettingsCard {
            ListItem(headlineContent={Text(entry)},trailingContent={IconButton(onClick={prefs.untrust(entry)}) {Icon(Icons.Outlined.DeleteOutline,ui(R.string.remove_trusted_domain_1_s, entry))}},colors=ListItemDefaults.colors(containerColor=androidx.compose.ui.graphics.Color.Transparent))
        }}}
    }
}
