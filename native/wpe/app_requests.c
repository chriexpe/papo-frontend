/* Native request policy only. No JS injection, script handler, or app API bridge. */
#include <libsoup/soup.h>
#include <wpe/webkit-web-process-extension.h>

static gboolean request(
    WebKitWebPage* page, WebKitURIRequest* request, WebKitURIResponse* redirect, gpointer data)
{
    (void)page;
    (void)redirect;
    (void)data;
    SoupMessageHeaders* headers = webkit_uri_request_get_http_headers(request);
    if (headers)
        soup_message_headers_remove(headers, "Referer");
    return FALSE;
}
static void created(WebKitWebProcessExtension* extension, WebKitWebPage* page, gpointer data)
{
    (void)extension;
    (void)data;
    g_signal_connect(page, "send-request", G_CALLBACK(request), NULL);
    webkit_web_page_send_message_to_view(
        page, webkit_user_message_new("papo-native-request-policy-v1", NULL), NULL, NULL, NULL);
}
G_MODULE_EXPORT void webkit_web_process_extension_initialize(WebKitWebProcessExtension* extension)
{
    g_signal_connect(extension, "page-created", G_CALLBACK(created), NULL);
}
