// Native macOS WebEmbed bridge. All AppKit/WebKit calls run on the UI thread.
// Rust retains the bridge object and owns the callback context.
#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/message.h>

typedef struct {
    double x, y, width, height;
} PapoRect;

typedef void (*PapoEventFn)(void *context, int kind, const char *url);

enum { PapoOpenExternal = 1, PapoFailed = 2 };
static NSString *const PapoReferer = @"https://io.github.chriexpe.papo/";

static BOOL papo_safe_url(NSURL *url) {
    NSString *scheme = url.scheme.lowercaseString;
    return [scheme isEqualToString:@"https"] || [scheme isEqualToString:@"http"];
}

@interface PapoClipView : NSView
@property(nonatomic, copy) NSArray<NSValue *> *holes;
@end

@implementation PapoClipView
- (BOOL)isFlipped { return YES; }
- (NSView *)hitTest:(NSPoint)point {
    NSPoint local = [self convertPoint:point fromView:self.superview];
    for (NSValue *rect in self.holes) {
        if (NSPointInRect(local, rect.rectValue)) {
            // The browser is also non-interactive over egui's floating chrome.
            return nil;
        }
    }
    return [super hitTest:point];
}
@end

@interface PapoNavigation : NSObject <WKNavigationDelegate, WKUIDelegate>
@property(nonatomic, assign) void *context;
@property(nonatomic, assign) PapoEventFn callback;
@property(nonatomic, assign) BOOL firstDocumentLoaded;
@property(nonatomic, strong) NSURL *lockedURL;
@end

@implementation PapoNavigation
- (void)emit:(int)kind url:(NSURL *)url {
    if (self.callback) {
        self.callback(self.context, kind, url ? url.absoluteString.UTF8String : NULL);
    }
}
- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationAction:(WKNavigationAction *)action
    decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    (void)webView;
    NSURL *url = action.request.URL;
    // Never let local files, custom URL schemes or javascript: escape the
    // application's explicit external-link security policy.
    if (!papo_safe_url(url)) {
        // WebKit uses about:blank when setting up iframes internally.
        BOOL blank = [url.absoluteString isEqualToString:@"about:blank"];
        decisionHandler(blank && !action.targetFrame.isMainFrame
                            ? WKNavigationActionPolicyAllow
                            : WKNavigationActionPolicyCancel);
        return;
    }

    BOOL topLevel = action.targetFrame == nil || action.targetFrame.isMainFrame;
    NSNumber *port=url.port ?: @([url.scheme.lowercaseString isEqualToString:@"https"] ? 443 : 80);
    NSNumber *lockedPort=self.lockedURL.port ?: @([self.lockedURL.scheme.lowercaseString isEqualToString:@"https"] ? 443 : 80);
    BOOL same=[url.scheme.lowercaseString isEqualToString:self.lockedURL.scheme.lowercaseString] && [url.host.lowercaseString isEqualToString:self.lockedURL.host.lowercaseString] && [port isEqualToNumber:lockedPort];
    if (topLevel && (!same || action.targetFrame==nil)) {
        [self emit:PapoOpenExternal url:url];
        decisionHandler(WKNavigationActionPolicyCancel);
        return;
    }
    // In-frame navigation (including scripted redirects) remains in WebKit.
    decisionHandler(WKNavigationActionPolicyAllow);
}
- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation {
    (void)webView; (void)navigation;
    self.firstDocumentLoaded = YES;
}
- (void)webView:(WKWebView *)webView
    didFailProvisionalNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    (void)webView; (void)navigation;
    if (error.code != NSURLErrorCancelled) [self emit:PapoFailed url:nil];
}
- (void)webViewWebContentProcessDidTerminate:(WKWebView *)webView {
    (void)webView;
    [self emit:PapoFailed url:nil];
}
- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
    forNavigationAction:(WKNavigationAction *)action
    windowFeatures:(WKWindowFeatures *)windowFeatures {
    (void)webView; (void)configuration; (void)windowFeatures;
    // window.open/scripted popups must never create a second native window.
    if (action.navigationType == WKNavigationTypeLinkActivated && papo_safe_url(action.request.URL)) {
        [self emit:PapoOpenExternal url:action.request.URL];
    }
    return nil;
}
- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationResponse:(WKNavigationResponse *)response
    decisionHandler:(void (^)(WKNavigationResponsePolicy))decisionHandler {
    (void)webView;
    // Third-party embeds may display media, but never initiate downloads.
    decisionHandler(response.canShowMIMEType
        ? WKNavigationResponsePolicyAllow : WKNavigationResponsePolicyCancel);
}
- (void)webView:(WKWebView *)webView
    requestMediaCapturePermissionForOrigin:(WKSecurityOrigin *)origin
    initiatedByFrame:(WKFrameInfo *)frame type:(WKMediaCaptureType)type
    decisionHandler:(void (^)(WKPermissionDecision))decisionHandler API_AVAILABLE(macos(12.0)) {
    (void)webView; (void)origin; (void)frame; (void)type;
    // Camera/microphone are for Papo's native GStreamer call, not the embed.
    decisionHandler(WKPermissionDecisionDeny);
}
@end

@interface PapoWebHost : NSObject
@property(nonatomic, strong) PapoClipView *clip;
@property(nonatomic, strong) WKWebView *webview;
@property(nonatomic, strong) PapoNavigation *navigation;
@property(nonatomic, weak) NSView *parent;
@end
@implementation PapoWebHost
@end

// The parent is a live winit NSView, captured from the raw AppKit handle.
void *papo_wk_create(void *parentView, const char *urlText,
                     void *context, PapoEventFn eventCallback) {
    @autoreleasepool {
        if (![NSThread isMainThread] || !parentView || !urlText) return NULL;
        NSView *view = (__bridge NSView *)parentView;
        NSView *parent = view.window.contentView;
        if (!parent) return NULL;

        NSString *urlString = [NSString stringWithUTF8String:urlText];
        NSURL *url = [NSURL URLWithString:urlString];
        if (!papo_safe_url(url)) return NULL;

        PapoWebHost *host = [PapoWebHost new];
        host.parent = parent;
        host.clip = [[PapoClipView alloc] initWithFrame:NSMakeRect(0, 0, 1, 1)];
        host.clip.holes = @[];
        host.clip.wantsLayer = YES;
        host.clip.layer.masksToBounds = YES;
        host.clip.hidden = YES;

        WKWebViewConfiguration *config = [WKWebViewConfiguration new];
        config.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
        config.preferences.javaScriptCanOpenWindowsAutomatically = NO;
        host.webview = [[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, 1, 1)
                                         configuration:config];
        host.navigation = [PapoNavigation new];
        host.navigation.context = context;
        host.navigation.callback = eventCallback;
        host.navigation.lockedURL = url;
        host.webview.navigationDelegate = host.navigation;
        host.webview.UIDelegate = host.navigation;
        host.webview.allowsBackForwardNavigationGestures = NO;
        host.webview.allowsMagnification = NO;

        [host.clip addSubview:host.webview];
        [parent addSubview:host.clip positioned:NSWindowAbove relativeTo:nil];

        NSMutableURLRequest *request = [NSMutableURLRequest requestWithURL:url];
        [request setValue:PapoReferer forHTTPHeaderField:@"Referer"];
        [host.webview loadRequest:request];
        return (__bridge_retained void *)host;
    }
}

void papo_wk_present(void *opaque, PapoRect full, PapoRect visible,
                     const PapoRect *cuts, size_t cutCount) {
    @autoreleasepool {
        if (!opaque || ![NSThread isMainThread]) return;
        PapoWebHost *host = (__bridge PapoWebHost *)opaque;
        NSView *parent = host.parent;
        if (!parent || !parent.window || visible.width <= 0 || visible.height <= 0) {
            host.clip.hidden = YES;
            return;
        }
        // Egui's viewport is in physical pixels. AppKit positions views in
        // backing-independent points, and typically uses bottom-left origin.
        CGFloat scale = parent.window.backingScaleFactor;
        if (scale <= 0) scale = 1;
        NSRect frame = NSMakeRect(visible.x / scale, visible.y / scale,
                                  visible.width / scale, visible.height / scale);
        if (!parent.isFlipped) {
            frame.origin.y = NSMaxY(parent.bounds) - NSMaxY(frame);
        }
        [host.clip setFrame:frame];
        [host.webview setFrame:NSMakeRect((full.x - visible.x) / scale,
                                          (full.y - visible.y) / scale,
                                          full.width / scale,
                                          full.height / scale)];

        NSMutableArray<NSValue *> *holes = [NSMutableArray array];
        CGMutablePathRef shapePath = CGPathCreateMutable();
        CGPathAddRect(shapePath, NULL, CGRectMake(0, 0, frame.size.width, frame.size.height));
        for (size_t i = 0; cuts && i < cutCount; ++i) {
            PapoRect c = cuts[i];
            CGFloat left = MAX(0, (c.x - visible.x) / scale);
            CGFloat top = MAX(0, (c.y - visible.y) / scale);
            CGFloat right = MIN(frame.size.width, (c.x + c.width - visible.x) / scale);
            CGFloat bottom = MIN(frame.size.height, (c.y + c.height - visible.y) / scale);
            if (right <= left || bottom <= top) continue;
            NSRect hole = NSMakeRect(left, top, right - left, bottom - top);
            [holes addObject:[NSValue valueWithRect:hole]];
            // Shape masks use CoreAnimation's origin; keep cuts visually
            // aligned with the hit-test rectangles in flipped NSView space.
            CGRect maskRect = CGRectMake(left, frame.size.height - bottom,
                                         right - left, bottom - top);
            CGPathAddRect(shapePath, NULL, maskRect);
        }
        host.clip.holes = holes;
        CAShapeLayer *mask = [CAShapeLayer layer];
        mask.frame = CGRectMake(0, 0, frame.size.width, frame.size.height);
        mask.path = shapePath;
        mask.fillRule = kCAFillRuleEvenOdd;
        host.clip.layer.mask = mask;
        CGPathRelease(shapePath);

        host.clip.hidden = NO;
    }
}

void papo_wk_hide(void *opaque) {
    if (!opaque || ![NSThread isMainThread]) return;
    PapoWebHost *host = (__bridge PapoWebHost *)opaque;
    host.clip.hidden = YES;
}

int papo_wk_is_playing(void *opaque) {
    if (!opaque || ![NSThread isMainThread]) return -1;
    PapoWebHost *host = (__bridge PapoWebHost *)opaque;
    SEL selector = @selector(isPlayingAudio);
    if (![host.webview respondsToSelector:selector]) return -1;
    return ((BOOL (*)(id, SEL))objc_msgSend)(host.webview, selector) ? 1 : 0;
}

void papo_wk_destroy(void *opaque) {
    @autoreleasepool {
        if (!opaque) return;
        PapoWebHost *host = CFBridgingRelease(opaque);
        [host.webview stopLoading];
        host.webview.navigationDelegate = nil;
        host.webview.UIDelegate = nil;
        [host.webview removeFromSuperview];
        [host.clip removeFromSuperview];
        host.navigation.callback = NULL;
        host.navigation.context = NULL;
    }
}
