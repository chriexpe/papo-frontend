/*
 * Minimal WPEPlatform bridge for Papo WebEmbed.
 *
 * Frame export/custom-display code is adapted from waterui-browser-wpe
 * (water-rs/waterui commit 034057120bdade53c00ce2a63c1094172d9c01a2),
 * licensed MIT OR Apache-2.0.
 *
 * Unlike WaterUI's self-contained bridge this file targets the distro
 * wpe-webkit-2.0 / wpe-platform-2.0 API directly. It intentionally contains
 * no GTK-only WebsiteDataManager APIs, no libsoup cookie code, no injected JS
 * bridge and no bundled-runtime path overrides.
 */

#include "papo_wpe.h"

#include <dlfcn.h>
#include <drm_fourcc.h>
#include <fcntl.h>
#include <gio/gio.h>
#include <glib-object.h>
#include <glib.h>
#include <libsoup/soup.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <wpe/headless/wpe-headless.h>
#include <wpe/webkit.h>
#include <wpe/wpe-platform.h>

enum {
    WATER_WPE_EVENT_NAVIGATION_STARTED = 1,
    WATER_WPE_EVENT_LOADING = 2,
    WATER_WPE_EVENT_LOADED = 3,
    WATER_WPE_EVENT_REDIRECT = 4,
    WATER_WPE_EVENT_LOAD_FAILED = 5,
    WATER_WPE_EVENT_TLS_FAILED = 6,
};

struct WaterWpeRuntime {
    GMainContext* context;
    WPEDisplay* delegate;
    WPEDisplay* display;
};

struct WaterWpePage {
    WaterWpeRuntime* runtime;
    WebKitWebView* web_view;
    gboolean isolated;
    gboolean bundled;
    gboolean response_allowed;
    gboolean request_policy;
    char* origin;
    char* pending_uri;
    WPEView* view;
    WPEToplevel* toplevel;
    WaterWpeEventCallback event_callback;
    WaterWpeFrameCallback frame_callback;
    void* user_data;
    WaterWpeDestroyNotify destroy_user_data;
};

typedef struct {
    GMainContext* context;
    WPEView* view;
    WPEBuffer* buffer;
    gint references;
} WaterWpeFrameToken;

typedef struct {
    WaterWpeFrameToken* token;
    int release_fence_fd;
} WaterWpeRelease;

typedef struct _WaterView {
    WPEView parent_instance;
    WaterWpePage* page;
} WaterView;

typedef struct _WaterViewClass {
    WPEViewClass parent_class;
} WaterViewClass;

typedef struct _WaterToplevel {
    WPEToplevel parent_instance;
} WaterToplevel;

typedef struct _WaterToplevelClass {
    WPEToplevelClass parent_class;
} WaterToplevelClass;

typedef struct _WaterDisplay {
    WPEDisplay parent_instance;
    WPEDisplay* delegate;
    WPEBufferFormats* formats;
} WaterDisplay;

typedef struct _WaterDisplayClass {
    WPEDisplayClass parent_class;
} WaterDisplayClass;

#define WATER_TYPE_VIEW (water_view_get_type())
#define WATER_IS_VIEW(instance) (G_TYPE_CHECK_INSTANCE_TYPE((instance), WATER_TYPE_VIEW))

G_DEFINE_TYPE(WaterView, water_view, WPE_TYPE_VIEW)
G_DEFINE_TYPE(WaterToplevel, water_toplevel, WPE_TYPE_TOPLEVEL)
G_DEFINE_TYPE(WaterDisplay, water_display, WPE_TYPE_DISPLAY)

static WaterWpeFrameToken* water_wpe_frame_token_ref(WaterWpeFrameToken* token)
{
    g_atomic_int_inc(&token->references);
    return token;
}

static void water_wpe_frame_token_unref(WaterWpeFrameToken* token)
{
    if (!g_atomic_int_dec_and_test(&token->references))
        return;
    g_object_unref(token->buffer);
    g_object_unref(token->view);
    g_main_context_unref(token->context);
    g_free(token);
}

static gboolean water_wpe_frame_presented_on_main(gpointer user_data)
{
    WaterWpeFrameToken* token = user_data;
    wpe_view_buffer_rendered(token->view, token->buffer);
    water_wpe_frame_token_unref(token);
    return G_SOURCE_REMOVE;
}

static gboolean water_wpe_frame_released_on_main(gpointer user_data)
{
    WaterWpeRelease* release = user_data;
    if (release->release_fence_fd >= 0)
        wpe_buffer_set_release_fence(release->token->buffer, release->release_fence_fd);
    wpe_view_buffer_released(release->token->view, release->token->buffer);
    water_wpe_frame_token_unref(release->token);
    g_free(release);
    return G_SOURCE_REMOVE;
}

static void water_view_toplevel_changed(WPEView* view)
{
    WPEToplevel* toplevel = wpe_view_get_toplevel(view);
    if (!toplevel) {
        wpe_view_unmap(view);
        return;
    }
    int width = 0;
    int height = 0;
    wpe_toplevel_get_size(toplevel, &width, &height);
    if (width > 0 && height > 0)
        wpe_view_resized(view, width, height);
    wpe_view_map(view);
}

static void water_view_constructed(GObject* object)
{
    G_OBJECT_CLASS(water_view_parent_class)->constructed(object);
    g_signal_connect_swapped(
        object, "notify::toplevel", G_CALLBACK(water_view_toplevel_changed), object);
}

static gboolean water_view_render_buffer(WPEView* view, WPEBuffer* buffer,
    const WPERectangle* damage_rects, guint n_damage_rects, GError** error)
{
    (void)damage_rects;
    (void)n_damage_rects;
    (void)error;

    WaterWpePage* page = ((WaterView*)view)->page;
    g_assert(page != NULL);
    g_assert(WPE_IS_BUFFER_DMA_BUF(buffer));

    WPEBufferDMABuf* dma_buf = WPE_BUFFER_DMA_BUF(buffer);
    guint32 n_planes = wpe_buffer_dma_buf_get_n_planes(dma_buf);
    g_assert_cmpuint(n_planes, ==, 1);
    g_assert_true(wpe_buffer_dma_buf_get_modifier(dma_buf) == DRM_FORMAT_MOD_LINEAR);

    WaterWpeFrameToken* token = g_new0(WaterWpeFrameToken, 1);
    token->context = g_main_context_ref(page->runtime->context);
    token->view = g_object_ref(view);
    token->buffer = g_object_ref(buffer);
    token->references = 1;

    WaterWpeFrame frame = {
        .token = token,
        .width = (uint32_t)wpe_buffer_get_width(buffer),
        .height = (uint32_t)wpe_buffer_get_height(buffer),
        .format = wpe_buffer_dma_buf_get_format(dma_buf),
        .modifier = wpe_buffer_dma_buf_get_modifier(dma_buf),
        .n_planes = n_planes,
        .fds = { -1, -1, -1, -1 },
        .rendering_fence_fd = wpe_buffer_take_rendering_fence(buffer),
    };

    for (guint32 plane = 0; plane < n_planes; ++plane) {
        int source_fd = wpe_buffer_dma_buf_get_fd(dma_buf, plane);
        frame.fds[plane] = fcntl(source_fd, F_DUPFD_CLOEXEC, 0);
        g_assert_cmpint(frame.fds[plane], >=, 0);
        frame.offsets[plane] = wpe_buffer_dma_buf_get_offset(dma_buf, plane);
        frame.strides[plane] = wpe_buffer_dma_buf_get_stride(dma_buf, plane);
    }

    page->frame_callback(page->user_data, &frame);
    return TRUE;
}

static void water_view_class_init(WaterViewClass* klass)
{
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    object_class->constructed = water_view_constructed;
    WPEViewClass* view_class = WPE_VIEW_CLASS(klass);
    view_class->render_buffer = water_view_render_buffer;
}

static void water_view_init(WaterView* view) { view->page = NULL; }

static void water_toplevel_constructed(GObject* object)
{
    G_OBJECT_CLASS(water_toplevel_parent_class)->constructed(object);
    wpe_toplevel_state_changed(WPE_TOPLEVEL(object), WPE_TOPLEVEL_STATE_ACTIVE);
}

static gboolean water_view_resize_to_toplevel(
    WPEToplevel* toplevel, WPEView* view, gpointer user_data)
{
    (void)user_data;
    int width = 0;
    int height = 0;
    wpe_toplevel_get_size(toplevel, &width, &height);
    if (width > 0 && height > 0)
        wpe_view_resized(view, width, height);
    return FALSE;
}

static gboolean water_toplevel_resize(WPEToplevel* toplevel, int width, int height)
{
    wpe_toplevel_resized(toplevel, width, height);
    // A custom toplevel does not resize its views by itself: without this
    // the WebView keeps WPE's default 1024x768 viewport, which shrinks the
    // page into Papo's rectangle and misplaces every pointer event.
    wpe_toplevel_foreach_view(toplevel, water_view_resize_to_toplevel, NULL);
    return TRUE;
}

static void water_toplevel_class_init(WaterToplevelClass* klass)
{
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    object_class->constructed = water_toplevel_constructed;
    WPEToplevelClass* toplevel_class = WPE_TOPLEVEL_CLASS(klass);
    toplevel_class->resize = water_toplevel_resize;
}

static void water_toplevel_init(WaterToplevel* toplevel) { (void)toplevel; }

static gboolean water_display_connect(WPEDisplay* display, GError** error)
{
    (void)display;
    (void)error;
    return TRUE;
}

static WPEView* water_display_create_view(WPEDisplay* display)
{
    return WPE_VIEW(g_object_new(water_view_get_type(), "display", display, NULL));
}

static WPEToplevel* water_display_create_toplevel(WPEDisplay* display, guint max_views)
{
    return WPE_TOPLEVEL(
        g_object_new(water_toplevel_get_type(), "display", display, "max-views", max_views, NULL));
}

static gpointer water_display_get_egl_display(WPEDisplay* display, GError** error)
{
    return wpe_display_get_egl_display(((WaterDisplay*)display)->delegate, error);
}

static WPEDRMDevice* water_display_get_drm_device(WPEDisplay* display)
{
    return wpe_display_get_drm_device(((WaterDisplay*)display)->delegate);
}

static WPEBufferFormats* water_display_get_preferred_buffer_formats(WPEDisplay* display)
{
    // The display vfunc transfers a reference; the public getter returns borrowed.
    return g_object_ref(((WaterDisplay*)display)->formats);
}

static gboolean water_display_use_explicit_sync(WPEDisplay* display)
{
    (void)display;
    return TRUE;
}

static void water_display_dispose(GObject* object)
{
    WaterDisplay* display = (WaterDisplay*)object;
    g_clear_object(&display->formats);
    g_clear_object(&display->delegate);
    G_OBJECT_CLASS(water_display_parent_class)->dispose(object);
}

static void water_display_class_init(WaterDisplayClass* klass)
{
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = water_display_dispose;

    WPEDisplayClass* display_class = WPE_DISPLAY_CLASS(klass);
    display_class->connect = water_display_connect;
    display_class->create_view = water_display_create_view;
    display_class->create_toplevel = water_display_create_toplevel;
    display_class->get_egl_display = water_display_get_egl_display;
    display_class->get_drm_device = water_display_get_drm_device;
    display_class->get_preferred_buffer_formats = water_display_get_preferred_buffer_formats;
    display_class->use_explicit_sync = water_display_use_explicit_sync;
}

static void water_display_init(WaterDisplay* display)
{
    display->delegate = NULL;
    display->formats = NULL;
}

static void water_wpe_load_changed(
    WebKitWebView* web_view, WebKitLoadEvent event, WaterWpePage* page)
{
    const char* uri = webkit_web_view_get_uri(web_view);
    if (!uri)
        uri = "about:blank";

    switch (event) {
    case WEBKIT_LOAD_STARTED:
        if (page->isolated)
            page->response_allowed = FALSE;
        page->event_callback(page->user_data, WATER_WPE_EVENT_NAVIGATION_STARTED, uri, "", 0);
        break;
    case WEBKIT_LOAD_FINISHED:
        if (page->isolated && (!page->request_policy || !page->response_allowed))
            break;
        page->event_callback(page->user_data, WATER_WPE_EVENT_LOADED, uri, "", 0);
        break;
    default:
        break;
    }
}

static gboolean water_wpe_load_failed(WebKitWebView* web_view, WebKitLoadEvent event,
    const char* failing_uri, GError* error, WaterWpePage* page)
{
    (void)web_view;
    (void)event;
    (void)failing_uri;
    if (g_error_matches(error, WEBKIT_NETWORK_ERROR, WEBKIT_NETWORK_ERROR_CANCELLED)
        || g_error_matches(error, WEBKIT_POLICY_ERROR,
            WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE))
        return TRUE;
    page->event_callback(page->user_data, WATER_WPE_EVENT_LOAD_FAILED, error->message, "", 0);
    return FALSE;
}

static gboolean water_wpe_tls_failed(WebKitWebView* web_view, const char* failing_uri,
    GTlsCertificate* certificate, GTlsCertificateFlags errors, WaterWpePage* page)
{
    (void)web_view;
    (void)certificate;
    char* message = g_strdup_printf("TLS certificate errors: 0x%x", errors);
    page->event_callback(page->user_data, WATER_WPE_EVENT_TLS_FAILED, failing_uri, message, 0);
    g_free(message);
    return FALSE;
}

uint32_t water_wpe_abi_version(void) { return WATER_WPE_ABI_VERSION; }

WaterWpeRuntime* water_wpe_runtime_new(char** error)
{
    g_assert(error != NULL);
    *error = NULL;

    WaterWpeRuntime* runtime = g_new0(WaterWpeRuntime, 1);
    runtime->context = g_main_context_ref_thread_default();
    runtime->delegate = wpe_display_headless_new();

    GError* display_error = NULL;
    if (!wpe_display_connect(runtime->delegate, &display_error)) {
        *error = g_strdup(display_error->message);
        g_error_free(display_error);
        g_object_unref(runtime->delegate);
        g_main_context_unref(runtime->context);
        g_free(runtime);
        return NULL;
    }

    WaterDisplay* display = (WaterDisplay*)g_object_new(water_display_get_type(), NULL);
    display->delegate = g_object_ref(runtime->delegate);

    WPEDRMDevice* device = wpe_display_get_drm_device(runtime->delegate);
    WPEBufferFormatsBuilder* builder = wpe_buffer_formats_builder_new(device);
    wpe_buffer_formats_builder_append_group(builder, device, WPE_BUFFER_FORMAT_USAGE_RENDERING);
    wpe_buffer_formats_builder_append_format(builder, DRM_FORMAT_ARGB8888, DRM_FORMAT_MOD_LINEAR);
    wpe_buffer_formats_builder_append_format(builder, DRM_FORMAT_XRGB8888, DRM_FORMAT_MOD_LINEAR);
    display->formats = wpe_buffer_formats_builder_end(builder);
    wpe_buffer_formats_builder_unref(builder);

    // Papo forwards a desktop mouse and keyboard. Without this WebKit reports
    // (hover: none) / (pointer: none) and YouTube falls back to its touch
    // player, where hover controls such as the volume slider never appear.
    // WebKit reads these capabilities from the primary display, not from the
    // WebView's own display, so Papo's display must also become primary.
    wpe_display_set_available_input_devices(WPE_DISPLAY(display),
        WPE_AVAILABLE_INPUT_DEVICE_MOUSE | WPE_AVAILABLE_INPUT_DEVICE_KEYBOARD);
    wpe_display_set_primary(WPE_DISPLAY(display));

    runtime->display = WPE_DISPLAY(display);
    return runtime;
}

void water_wpe_runtime_free(WaterWpeRuntime* runtime)
{
    g_assert(runtime != NULL);
    while (g_main_context_iteration(runtime->context, FALSE)) { }
    g_object_unref(runtime->display);
    g_object_unref(runtime->delegate);
    g_main_context_unref(runtime->context);
    g_free(runtime);
}

bool water_wpe_runtime_iteration(WaterWpeRuntime* runtime)
{
    g_assert(runtime != NULL);
    return g_main_context_iteration(runtime->context, FALSE);
}

void water_wpe_string_free(char* string) { g_free(string); }

static char* app_origin(const char* uri)
{
    GUri* url = g_uri_parse(uri, G_URI_FLAGS_NONE, NULL);
    if (!url)
        return NULL;
    const char *scheme = g_uri_get_scheme(url), *host = g_uri_get_host(url);
    if (!scheme || !host
        || (g_ascii_strcasecmp(scheme, "https") && g_ascii_strcasecmp(scheme, "http"))
        || g_uri_get_userinfo(url)) {
        g_uri_unref(url);
        return NULL;
    }
    int port = g_uri_get_port(url);
    if (port < 0)
        port = !g_ascii_strcasecmp(scheme, "https") ? 443 : 80;
    char* origin = g_strdup_printf("%s://%s:%d", scheme, host, port);
    for (char* c = origin; *c; c++)
        *c = g_ascii_tolower(*c);
    g_uri_unref(url);
    return origin;
}
static gboolean app_refuses(const char* xfo, const char* csp)
{
    char* x = g_ascii_strdown(xfo ? xfo : "", -1);
    gboolean denied = strstr(x, "deny") || strstr(x, "sameorigin");
    g_free(x);
    char* policy = g_ascii_strdown(csp ? csp : "", -1);
    char** directives = g_strsplit_set(policy, ";,", -1);
    for (int i = 0; directives[i]; i++) {
        char* d = g_strstrip(directives[i]);
        if (g_str_has_prefix(d, "frame-ancestors") && (!d[15] || g_ascii_isspace(d[15]))
            && strcmp(g_strstrip(d + 15), "*"))
            denied = TRUE;
    }
    g_strfreev(directives);
    g_free(policy);
    return denied;
}
static gboolean app_native_policy(
    WebKitWebView* view, WebKitUserMessage* message, WaterWpePage* page)
{
    (void)view;
    if (!strcmp(webkit_user_message_get_name(message), "papo-native-request-policy-v1")) {
        page->request_policy = TRUE;
        char* pending = g_steal_pointer(&page->pending_uri);
        if (pending) {
            water_wpe_page_load_uri(page, pending);
            g_free(pending);
        }
        return TRUE;
    }
    return FALSE;
}
static gboolean app_decide(WebKitWebView* view, WebKitPolicyDecision* decision,
    WebKitPolicyDecisionType type, WaterWpePage* page)
{
    (void)view;
    if (type == WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION
        || type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
        WebKitNavigationAction* action = webkit_navigation_policy_decision_get_navigation_action(
            WEBKIT_NAVIGATION_POLICY_DECISION(decision));
        const char* uri = webkit_uri_request_get_uri(webkit_navigation_action_get_request(action));
        if (page->isolated && !page->response_allowed && !strcmp(uri, "about:blank"))
            return FALSE;
        char* origin = app_origin(uri);
        gboolean allow = type != WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION && origin
            && page->origin && !strcmp(origin, page->origin);
        g_free(origin);
        if (!allow) {
            webkit_policy_decision_ignore(decision);
            page->event_callback(page->user_data, 7, uri, "", 0);
            return TRUE;
        }
    } else if (page->isolated && type == WEBKIT_POLICY_DECISION_TYPE_RESPONSE) {
        WebKitResponsePolicyDecision* response = WEBKIT_RESPONSE_POLICY_DECISION(decision);
        if (webkit_response_policy_decision_is_main_frame_main_resource(response)) {
            WebKitURIResponse* uri = webkit_response_policy_decision_get_response(response);
            SoupMessageHeaders* headers = webkit_uri_response_get_http_headers(uri);
            const char* csp = headers
                ? soup_message_headers_get_list(headers, "Content-Security-Policy")
                : NULL;
            gboolean sandbox = csp && strstr(csp, "sandbox allow-scripts allow-forms")
                && !strstr(csp, "allow-same-origin");
            page->response_allowed = headers && webkit_uri_response_get_status_code(uri) < 400
                && !app_refuses(soup_message_headers_get_list(headers, "X-Frame-Options"), csp)
                && (!page->bundled || sandbox);
            if (!page->response_allowed) {
                webkit_policy_decision_ignore(decision);
                page->event_callback(page->user_data, 5, "frame policy", "", 0);
                return TRUE;
            }
        }
    }
    return FALSE;
}
static WebKitWebView* app_popup(WebKitWebView* v, WebKitNavigationAction* a, WaterWpePage* p)
{
    (void)v;
    p->event_callback(p->user_data, 7,
        webkit_uri_request_get_uri(webkit_navigation_action_get_request(a)), "", 0);
    return NULL;
}
static gboolean app_permission(WebKitWebView* v, WebKitPermissionRequest* r, gpointer p)
{
    (void)v;
    (void)p;
    webkit_permission_request_deny(r);
    return TRUE;
}
static gboolean app_auth(WebKitWebView* v, WebKitAuthenticationRequest* r, gpointer p)
{
    (void)v;
    (void)p;
    webkit_authentication_request_cancel(r);
    return TRUE;
}
static void app_download(WebKitNetworkSession* s, WebKitDownload* d, gpointer p)
{
    (void)s;
    (void)p;
    webkit_download_cancel(d);
}

/* WPE 2.54's lazy ATK application root snapshots existing toplevels, then
 * constructed() registers the first one again. Prime the root before creating
 * that first toplevel, otherwise its duplicate raw pointer survives disposal.
 * Optional ATK: builds without it do not need an accessibility workaround. */
static void water_wpe_prime_accessibility(gboolean enumerate)
{
    GModule* module = g_module_open("libatk-1.0.so.0", G_MODULE_BIND_LAZY | G_MODULE_BIND_LOCAL);
    if (!module)
        return;
    gpointer symbol = NULL;
    if (g_module_symbol(module, "atk_get_root", &symbol)) {
        gpointer (*get_root)(void) = NULL;
        G_STATIC_ASSERT(sizeof(get_root) == sizeof(symbol));
        memcpy(&get_root, &symbol, sizeof(get_root));
        gpointer root = get_root();
        if (root && enumerate) {
            gint (*n_children)(gpointer) = NULL;
            gpointer (*ref_child)(gpointer, gint) = NULL;
            if (g_module_symbol(module, "atk_object_get_n_accessible_children", &symbol))
                memcpy(&n_children, &symbol, sizeof(n_children));
            if (g_module_symbol(module, "atk_object_ref_accessible_child", &symbol))
                memcpy(&ref_child, &symbol, sizeof(ref_child));
            if (n_children && ref_child)
                for (gint i = 0, n = n_children(root); i < n; i++) {
                    gpointer child = ref_child(root, i);
                    if (child)
                        g_object_unref(child);
                }
        }
    }
    g_module_close(module);
}

static WaterWpePage* water_wpe_page_create(WaterWpeRuntime* runtime,
    WaterWpeEventCallback event_callback, WaterWpeFrameCallback frame_callback,
    WaterWpeMessageCallback message_callback, void* user_data,
    WaterWpeDestroyNotify destroy_user_data, char** error, bool isolated, bool bundled)
{
    g_assert(runtime != NULL);
    g_assert(event_callback != NULL);
    g_assert(frame_callback != NULL);
    g_assert(destroy_user_data != NULL);
    g_assert(error != NULL);
    (void)message_callback;
    *error = NULL;

    WaterWpePage* page = g_new0(WaterWpePage, 1);
    page->runtime = runtime;
    page->event_callback = event_callback;
    page->frame_callback = frame_callback;
    page->user_data = user_data;
    page->destroy_user_data = destroy_user_data;

    page->isolated = isolated;
    page->bundled = bundled;
    if (isolated) {
        Dl_info location;
        if (!dladdr((const void*)(uintptr_t)water_wpe_abi_version, &location)
            || !location.dli_fname) {
            *error = g_strdup("WPE policy module path unavailable");
            destroy_user_data(user_data);
            g_free(page);
            return NULL;
        }
        char* libdir = g_path_get_dirname(location.dli_fname);
        char* extensions = g_build_filename(libdir, "extensions", NULL);
        char* module = g_build_filename(extensions, "libpapo_app_requests.so", NULL);
        gboolean exists = g_file_test(module, G_FILE_TEST_IS_REGULAR);
        g_free(libdir);
        g_free(module);
        if (!exists) {
            g_free(extensions);
            *error = g_strdup("WPE native request policy missing");
            destroy_user_data(user_data);
            g_free(page);
            return NULL;
        }
        WebKitWebContext* context = webkit_web_context_new();
        webkit_web_context_set_web_process_extensions_directory(context, extensions);
        g_free(extensions);
        WebKitNetworkSession* session = webkit_network_session_new_ephemeral();
        if (!webkit_network_session_is_ephemeral(session)) {
            g_object_unref(session);
            g_object_unref(context);
            *error = g_strdup("WPE session is not ephemeral");
            destroy_user_data(user_data);
            g_free(page);
            return NULL;
        }
        webkit_network_session_set_persistent_credential_storage_enabled(session, FALSE);
        page->web_view = WEBKIT_WEB_VIEW(g_object_new(WEBKIT_TYPE_WEB_VIEW, "display",
            runtime->display, "network-session", session, "web-context", context, NULL));
        g_object_unref(context);
        g_signal_connect(
            page->web_view, "user-message-received", G_CALLBACK(app_native_policy), page);
        g_signal_connect(session, "download-started", G_CALLBACK(app_download), NULL);
        g_object_unref(session);
        g_signal_connect(page->web_view, "permission-request", G_CALLBACK(app_permission), NULL);
        g_signal_connect(page->web_view, "authenticate", G_CALLBACK(app_auth), NULL);
    } else {
        page->web_view = WEBKIT_WEB_VIEW(
            g_object_new(WEBKIT_TYPE_WEB_VIEW, "display", runtime->display, NULL));
    }
    g_signal_connect(page->web_view, "decide-policy", G_CALLBACK(app_decide), page);
    g_signal_connect(page->web_view, "create", G_CALLBACK(app_popup), page);

    // Equibop/Discord runs embeds under Chromium and explicitly presents a
    // Chrome-style Linux UA. Match that browser capability identity rather
    // than altering iframe geometry or page zoom.
    WebKitSettings* settings = webkit_web_view_get_settings(page->web_view);
    webkit_settings_set_user_agent(settings,
        "Mozilla/5.0 (X11; Linux x86_64) "
        "AppleWebKit/537.36 (KHTML, like Gecko) "
        "Chrome/150.0.0.0 Safari/537.36");

    page->view = webkit_web_view_get_wpe_view(page->web_view);
    if (!WATER_IS_VIEW(page->view)) {
        *error = g_strdup("WPE WebView did not create the Papo WPEView");
        g_object_unref(page->web_view);
        destroy_user_data(user_data);
        g_free(page);
        return NULL;
    }

    ((WaterView*)page->view)->page = page;
    water_wpe_prime_accessibility(FALSE);
    page->toplevel = wpe_display_create_toplevel(runtime->display, 1);
    wpe_view_set_toplevel(page->view, page->toplevel);
    wpe_toplevel_resize(page->toplevel, 1, 1);
    wpe_view_set_visible(page->view, TRUE);

    g_signal_connect(page->web_view, "load-changed", G_CALLBACK(water_wpe_load_changed), page);
    g_signal_connect(page->web_view, "load-failed", G_CALLBACK(water_wpe_load_failed), page);
    g_signal_connect(
        page->web_view, "load-failed-with-tls-errors", G_CALLBACK(water_wpe_tls_failed), page);

    return page;
}

WaterWpePage* water_wpe_page_new(WaterWpeRuntime* r, WaterWpeEventCallback e,
    WaterWpeFrameCallback f, WaterWpeMessageCallback m, void* u, WaterWpeDestroyNotify d,
    char** error)
{
    return water_wpe_page_create(r, e, f, m, u, d, error, false, false);
}
WaterWpePage* water_wpe_page_new_isolated(WaterWpeRuntime* r, WaterWpeEventCallback e,
    WaterWpeFrameCallback f, WaterWpeMessageCallback m, void* u, WaterWpeDestroyNotify d,
    char** error, bool bundled)
{
    return water_wpe_page_create(r, e, f, m, u, d, error, true, bundled);
}

void water_wpe_page_free(WaterWpePage* page)
{
    g_assert(page != NULL);
    // The ATK root may have been created asynchronously after construction.
    // Materialize child accessibles while all its raw toplevel pointers are live;
    // WPE then removes those children during the toplevel's dispose().
    water_wpe_prime_accessibility(TRUE);
    g_signal_handlers_disconnect_by_data(page->web_view, page);
    webkit_web_view_stop_loading(page->web_view);
    ((WaterView*)page->view)->page = NULL;
    wpe_view_closed(page->view);
    wpe_toplevel_closed(page->toplevel);
    g_object_unref(page->toplevel);
    g_object_unref(page->web_view);
    g_free(page->origin);
    g_free(page->pending_uri);
    page->destroy_user_data(page->user_data);
    g_free(page);
}

void water_wpe_page_load_uri(WaterWpePage* page, const char* uri)
{
    g_assert(page != NULL);
    g_assert(uri != NULL);

    if (page->isolated && !page->request_policy) {
        g_free(page->pending_uri);
        page->pending_uri = g_strdup(uri);
        // Bootstrap only a non-network document until the native request hook acknowledges.
        webkit_web_view_load_uri(page->web_view, "about:blank");
        return;
    }
    // Load the real embed directly, just like a normal browser frame. YouTube
    // requires the embedding app identity in Referer; WebKitURIRequest exposes
    // the mutable HTTP headers, so there is no need for a synthetic wrapper
    // page, iframe scaling or viewport tricks.
    WebKitURIRequest* request = webkit_uri_request_new(uri);
    SoupMessageHeaders* headers = webkit_uri_request_get_http_headers(request);
    g_free(page->origin);
    page->origin = app_origin(uri);
    if (headers && !page->isolated)
        soup_message_headers_replace(headers, "Referer", "https://io.github.chriexpe.papo/");
    webkit_web_view_load_request(page->web_view, request);
    g_object_unref(request);
}

void water_wpe_page_stop(WaterWpePage* page)
{
    g_assert(page != NULL);
    webkit_web_view_stop_loading(page->web_view);
}

void water_wpe_page_resize(WaterWpePage* page, uint32_t width, uint32_t height, double scale)
{
    g_assert(page != NULL);
    g_assert_cmpuint(width, >, 0);
    g_assert_cmpuint(height, >, 0);
    g_assert_cmpfloat(scale, >, 0);
    wpe_toplevel_scale_changed(page->toplevel, scale);
    gboolean resized = wpe_toplevel_resize(page->toplevel, (int)width, (int)height);
    g_assert(resized);
}

void water_wpe_page_set_focus(WaterWpePage* page, bool focused)
{
    g_assert(page != NULL);
    if (focused)
        wpe_view_focus_in(page->view);
    else
        wpe_view_focus_out(page->view);
}

bool water_wpe_page_is_playing_audio(WaterWpePage* page)
{
    g_assert(page != NULL);
    return webkit_web_view_is_playing_audio(page->web_view);
}

void water_wpe_page_pointer_button(WaterWpePage* page, bool pressed, uint32_t button, double x,
    double y, uint32_t modifiers, uint32_t time_ms)
{
    WPEEventType type = pressed ? WPE_EVENT_POINTER_DOWN : WPE_EVENT_POINTER_UP;
    // WPE only accepts a non-zero press count on POINTER_DOWN; every other
    // type must carry zero. Passing the computed count on UP made the
    // constructor return NULL and the following wpe_view_event() abort.
    guint press_count
        = pressed ? wpe_view_compute_press_count(page->view, x, y, button, time_ms) : 0;
    WPEEvent* event = wpe_event_pointer_button_new(type, page->view, WPE_INPUT_SOURCE_MOUSE,
        time_ms, (WPEModifiers)modifiers, button, x, y, press_count);
    if (event == NULL)
        return;
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_page_pointer_move(WaterWpePage* page, double x, double y, double delta_x,
    double delta_y, uint32_t modifiers, uint32_t time_ms)
{
    WPEEvent* event = wpe_event_pointer_move_new(WPE_EVENT_POINTER_MOVE, page->view,
        WPE_INPUT_SOURCE_MOUSE, time_ms, (WPEModifiers)modifiers, x, y, delta_x, delta_y);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_page_scroll(WaterWpePage* page, double x, double y, double delta_x, double delta_y,
    bool precise, bool stopped, uint32_t modifiers, uint32_t time_ms)
{
    WPEEvent* event = wpe_event_scroll_new(page->view, WPE_INPUT_SOURCE_TOUCHPAD, time_ms,
        (WPEModifiers)modifiers, delta_x, delta_y, precise, stopped, x, y);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_frame_presented(void* user_data)
{
    WaterWpeFrameToken* token = user_data;
    g_main_context_invoke_full(token->context, G_PRIORITY_HIGH, water_wpe_frame_presented_on_main,
        water_wpe_frame_token_ref(token), NULL);
}

void water_wpe_frame_release(void* user_data, int release_fence_fd)
{
    WaterWpeFrameToken* token = user_data;
    WaterWpeRelease* release = g_new0(WaterWpeRelease, 1);
    release->token = token;
    release->release_fence_fd = release_fence_fd;
    g_main_context_invoke_full(
        token->context, G_PRIORITY_HIGH, water_wpe_frame_released_on_main, release, NULL);
    water_wpe_frame_token_unref(token);
}

void water_wpe_page_key(WaterWpePage* page, bool pressed, uint32_t keyval, uint32_t keycode, uint32_t modifiers)
{
    WPEEvent* event
        = wpe_event_keyboard_new(pressed ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP,
            page->view, WPE_INPUT_SOURCE_KEYBOARD, 0, (WPEModifiers)modifiers, keycode, keyval);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}
