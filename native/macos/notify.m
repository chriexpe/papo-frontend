// Native macOS local notifications. App UI and notification callback stay
// isolated from Rust runtime threads; every UserNotifications API call is
// scheduled on the main queue.
#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <UserNotifications/UserNotifications.h>
#import <dispatch/dispatch.h>

@interface PapoNotificationDelegate : NSObject <UNUserNotificationCenterDelegate>
@end
@implementation PapoNotificationDelegate
- (void)userNotificationCenter:(UNUserNotificationCenter *)center
       willPresentNotification:(UNNotification *)notification
         withCompletionHandler:(void (^)(UNNotificationPresentationOptions))completion {
    (void)center; (void)notification;
    // Show desktop notifications even if Papo itself is frontmost;
    // filtering/aggregation is owned by the shared Rust coordinator.
    completion(UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionSound);
}
@end

static PapoNotificationDelegate *papo_delegate;

void papo_notify_initialize(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
        if (!papo_delegate) {
            papo_delegate = [PapoNotificationDelegate new];
            center.delegate = papo_delegate;
        }
        [center requestAuthorizationWithOptions:(UNAuthorizationOptionAlert |
                                                 UNAuthorizationOptionSound |
                                                 UNAuthorizationOptionBadge)
                              completionHandler:^(BOOL granted, NSError *error) {
            if (error) NSLog(@"Papo notifications permission: %@", error);
            else if (!granted) NSLog(@"Papo notifications permission denied");
        }];
    });
}

void papo_notify_show(const char *title, const char *body, const char *tag) {
    if (!title || !body) return;
    NSString *titleText = [NSString stringWithUTF8String:title];
    NSString *bodyText = [NSString stringWithUTF8String:body];
    NSString *tagText = tag ? [NSString stringWithUTF8String:tag] : nil;
    if (!titleText || !bodyText) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        UNMutableNotificationContent *content = [UNMutableNotificationContent new];
        content.title = titleText;
        content.body = bodyText;
        content.sound = [UNNotificationSound defaultSound];
        // Stable channel tags replace previous pending notifications; no
        // unbounded per-message notifications from busy channels.
        NSString *identifier = tagText.length ? tagText : NSUUID.UUID.UUIDString;
        UNNotificationRequest *request =
            [UNNotificationRequest requestWithIdentifier:identifier
                                                 content:content
                                                 trigger:nil];
        [UNUserNotificationCenter.currentNotificationCenter
            addNotificationRequest:request
             withCompletionHandler:^(NSError *error) {
                if (error) NSLog(@"Papo notification delivery: %@", error);
             }];
    });
}
