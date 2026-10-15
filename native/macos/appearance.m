// AppKit system appearance and accent color for the shared Papo theme.
#import <AppKit/AppKit.h>
#include <stdint.h>

// Returns -1 for unknown, 0 for Aqua/light and 1 for dark Aqua.
// Accent RGB channels are initialized even if no convertible accent exists.
int papo_macos_system_appearance(uint8_t *red, uint8_t *green, uint8_t *blue) {
    @autoreleasepool {
        if (!red || !green || !blue || ![NSThread isMainThread]) return -1;
        *red = *green = *blue = 0;
        NSColor *accent = [[NSColor controlAccentColor]
                           colorUsingColorSpace:NSColorSpace.deviceRGBColorSpace];
        if (accent) {
            *red = (uint8_t)lround(MAX(0, MIN(1, accent.redComponent)) * 255);
            *green = (uint8_t)lround(MAX(0, MIN(1, accent.greenComponent)) * 255);
            *blue = (uint8_t)lround(MAX(0, MIN(1, accent.blueComponent)) * 255);
        }
        NSAppearance *appearance = NSApp.effectiveAppearance ?: NSAppearance.currentAppearance;
        NSString *matched = [appearance bestMatchFromAppearancesWithNames:
                                  @[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]];
        if ([matched isEqualToString:NSAppearanceNameDarkAqua]) return 1;
        if ([matched isEqualToString:NSAppearanceNameAqua]) return 0;
        return -1;
    }
}
