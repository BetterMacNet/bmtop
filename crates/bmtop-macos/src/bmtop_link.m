/* 网络链路 / 雷雳拓扑 / 屏幕 FPS 采集实现。逻辑移植自 mactop（MIT）。 */
#include "bmtop_link.h"

#import <CoreWLAN/CoreWLAN.h>

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <dlfcn.h>
#include <ifaddrs.h>
#include <mach/mach_time.h>
#include <net/if.h>
#include <net/if_media.h>
#include <stdatomic.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

/* ---- Ethernet 链路（纯 BSD）---- */
static uint64_t ifm_subtype_mbps(int subtype) {
    switch (subtype) {
    case IFM_10_T: return 10;
    case IFM_100_TX: return 100;
    case IFM_1000_T:
    case IFM_1000_SX:
    case IFM_1000_LX:
    case IFM_1000_CX: return 1000;
#ifdef IFM_2500_T
    case IFM_2500_T: return 2500;
#endif
#ifdef IFM_2500_SX
    case IFM_2500_SX: return 2500;
#endif
#ifdef IFM_5000_T
    case IFM_5000_T: return 5000;
#endif
#ifdef IFM_10G_T
    case IFM_10G_T: return 10000;
#endif
#ifdef IFM_10G_SR
    case IFM_10G_SR: return 10000;
#endif
#ifdef IFM_10G_LR
    case IFM_10G_LR: return 10000;
#endif
    default: return 0;
    }
}

size_t bmtop_read_ethernet_links(bmtop_eth_link *out, size_t capacity) {
    if (out == NULL || capacity == 0) return 0;
    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    if (sock < 0) return 0;
    struct ifaddrs *ifap = NULL;
    if (getifaddrs(&ifap) != 0) {
        close(sock);
        return 0;
    }
    size_t count = 0;
    for (struct ifaddrs *ifa = ifap; ifa != NULL && count < capacity; ifa = ifa->ifa_next) {
        if (ifa->ifa_addr == NULL || ifa->ifa_addr->sa_family != AF_LINK) continue;
        const char *name = ifa->ifa_name;
        if (strncmp(name, "en", 2) != 0) continue;
        int duplicate = 0;
        for (size_t i = 0; i < count; i++) {
            if (strcmp(out[i].name, name) == 0) {
                duplicate = 1;
                break;
            }
        }
        if (duplicate) continue;
        struct ifmediareq ifmr;
        memset(&ifmr, 0, sizeof(ifmr));
        strncpy(ifmr.ifm_name, name, sizeof(ifmr.ifm_name) - 1);
        if (ioctl(sock, SIOCGIFMEDIA, &ifmr) < 0) continue;       /* 多半是 Wi-Fi */
        if ((ifmr.ifm_status & IFM_AVALID) == 0) continue;
        if (IFM_TYPE(ifmr.ifm_active) != IFM_ETHER) continue;     /* 排除 IEEE80211 */
        bmtop_eth_link *link = &out[count++];
        memset(link, 0, sizeof(*link));
        strncpy(link->name, name, sizeof(link->name) - 1);
        link->link_up = (ifmr.ifm_status & IFM_ACTIVE) ? 1 : 0;
        if (link->link_up) link->speed_mbps = ifm_subtype_mbps(IFM_SUBTYPE(ifmr.ifm_active));
    }
    freeifaddrs(ifap);
    close(sock);
    return count;
}

/* ---- Wi-Fi 链路（CoreWLAN）----
 * 只读 interfaceName/transmitRate/serviceActive/activePHYMode。
 * 绝不读 ssid/bssid——那会挂上「定位服务」授权，CLI 拿不到只会得到 nil。 */
int bmtop_read_wifi_link(bmtop_wifi_link *out) {
    if (out == NULL) return -1;
    memset(out, 0, sizeof(*out));
    @autoreleasepool {
        CWWiFiClient *client = [CWWiFiClient sharedWiFiClient];
        if (client == nil) return -1;
        CWInterface *interface = [client interface];
        if (interface == nil) return -1;
        const char *name = [[interface interfaceName] UTF8String];
        if (name != NULL) strncpy(out->name, name, sizeof(out->name) - 1);
        out->tx_rate_mbps = (int32_t)[interface transmitRate];
        out->connected = [interface serviceActive] ? 1 : 0;
        NSInteger mode = [interface activePHYMode];
        const char *phy = "Unknown";
        const char *gen = "";
        switch (mode) {
        case 0: phy = "None"; break;
        case 1: phy = "802.11a"; gen = "Wi-Fi 2"; break;
        case 2: phy = "802.11b"; gen = "Wi-Fi 1"; break;
        case 3: phy = "802.11g"; gen = "Wi-Fi 3"; break;
        case 4: phy = "802.11n"; gen = "Wi-Fi 4"; break;
        case 5: phy = "802.11ac"; gen = "Wi-Fi 5"; break;
        case 6: phy = "802.11ax"; gen = "Wi-Fi 6"; break;
        case 7: phy = "802.11be"; gen = "Wi-Fi 7"; break;
        default: break;
        }
        strncpy(out->phy_mode, phy, sizeof(out->phy_mode) - 1);
        strncpy(out->generation, gen, sizeof(out->generation) - 1);
    }
    return 0;
}

/* ---- 雷雳拓扑（IOThunderboltSwitch）---- */
static int64_t registry_i64(CFDictionaryRef props, const char *key) {
    CFStringRef key_ref =
        CFStringCreateWithCString(kCFAllocatorDefault, key, kCFStringEncodingUTF8);
    CFNumberRef number = CFDictionaryGetValue(props, key_ref);
    CFRelease(key_ref);
    int64_t value = 0;
    if (number != NULL && CFGetTypeID(number) == CFNumberGetTypeID()) {
        CFNumberGetValue(number, kCFNumberSInt64Type, &value);
    }
    return value;
}

static void registry_string(CFDictionaryRef props, const char *key, char *out, size_t capacity) {
    out[0] = '\0';
    CFStringRef key_ref =
        CFStringCreateWithCString(kCFAllocatorDefault, key, kCFStringEncodingUTF8);
    CFStringRef value = CFDictionaryGetValue(props, key_ref);
    CFRelease(key_ref);
    if (value != NULL && CFGetTypeID(value) == CFStringGetTypeID()) {
        CFStringGetCString(value, out, capacity, kCFStringEncodingUTF8);
    }
}

/* depth>0 的设备沿 parent 链向上找 depth==0 的主机总线 UID。 */
static int64_t find_root_uid(io_object_t entry) {
    io_object_t current = entry;
    IOObjectRetain(current);
    int64_t root_uid = 0;
    for (int hops = 0; hops < 16 && root_uid == 0; hops++) {
        io_object_t parent = 0;
        kern_return_t rc = IORegistryEntryGetParentEntry(current, kIOServicePlane, &parent);
        IOObjectRelease(current);
        if (rc != kIOReturnSuccess) return 0;
        current = parent;
        CFMutableDictionaryRef props = NULL;
        if (IORegistryEntryCreateCFProperties(current, &props, kCFAllocatorDefault, 0) ==
            kIOReturnSuccess) {
            CFStringRef uid_key = CFSTR("UID");
            if (CFDictionaryContainsKey(props, uid_key) && registry_i64(props, "Depth") == 0) {
                root_uid = registry_i64(props, "UID");
            }
            CFRelease(props);
        }
    }
    IOObjectRelease(current);
    return root_uid;
}

/* 速度在第一个 IOThunderboltPort 子节点上。 */
static void read_port_speeds(io_object_t entry, int32_t *link_speed, int32_t *current_speed) {
    io_iterator_t children;
    if (IORegistryEntryGetChildIterator(entry, kIOServicePlane, &children) != kIOReturnSuccess) {
        return;
    }
    io_object_t child;
    while ((child = IOIteratorNext(children)) != 0) {
        io_name_t class_name;
        IOObjectGetClass(child, class_name);
        if (strstr(class_name, "IOThunderboltPort") != NULL) {
            CFMutableDictionaryRef props = NULL;
            if (IORegistryEntryCreateCFProperties(child, &props, kCFAllocatorDefault, 0) ==
                kIOReturnSuccess) {
                int64_t supported = registry_i64(props, "Supported Link Speed");
                int64_t current = registry_i64(props, "Current Link Speed");
                if (supported > 0) *link_speed = (int32_t)supported;
                if (current > 0) *current_speed = (int32_t)current;
                CFRelease(props);
            }
            IOObjectRelease(child);
            break;
        }
        IOObjectRelease(child);
    }
    IOObjectRelease(children);
}

size_t bmtop_read_tb_switches(bmtop_tb_switch *out, size_t capacity) {
    if (out == NULL || capacity == 0) return 0;
    io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault,
                                     IOServiceMatching("IOThunderboltSwitch"),
                                     &iterator) != kIOReturnSuccess) {
        return 0;
    }
    size_t count = 0;
    io_object_t entry;
    while ((entry = IOIteratorNext(iterator)) != 0 && count < capacity) {
        CFMutableDictionaryRef props = NULL;
        if (IORegistryEntryCreateCFProperties(entry, &props, kCFAllocatorDefault, 0) ==
            kIOReturnSuccess) {
            bmtop_tb_switch *item = &out[count++];
            memset(item, 0, sizeof(*item));
            item->uid = registry_i64(props, "UID");
            item->depth = (int32_t)registry_i64(props, "Depth");
            registry_string(props, "Device Vendor Name", item->vendor, sizeof(item->vendor));
            registry_string(props, "Device Model Name", item->device, sizeof(item->device));
            read_port_speeds(entry, &item->link_speed, &item->current_speed);
            if (item->depth > 0) item->parent_uid = find_root_uid(entry);
            CFRelease(props);
        }
        IOObjectRelease(entry);
    }
    IOObjectRelease(iterator);
    return count;
}

/* ---- 屏幕 FPS（CGDisplayStream，dlopen）---- */
typedef void *CGDisplayStreamRef;
typedef void (^dfps_handler)(int32_t status, uint64_t display_time, void *surface, void *update);
typedef CGDisplayStreamRef (*fn_create)(uint32_t display, size_t width, size_t height,
                                        int32_t pixel_format, CFDictionaryRef properties,
                                        dispatch_queue_t queue, dfps_handler handler);
typedef int32_t (*fn_start)(CGDisplayStreamRef stream);
typedef int32_t (*fn_stop)(CGDisplayStreamRef stream);
typedef size_t (*fn_drop_count)(void *update);
typedef uint32_t (*fn_main_display)(void);
typedef int (*fn_preflight)(void);

static CGDisplayStreamRef g_fps_stream = NULL;
static dispatch_queue_t g_fps_queue = NULL;
static _Atomic uint64_t g_fps_frames = 0;
static uint64_t g_fps_last_read = 0;

static void *cg_symbol(const char *name) {
    static void *handle = NULL;
    if (handle == NULL) {
        handle = dlopen(
            "/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics", RTLD_LAZY);
    }
    return handle != NULL ? dlsym(handle, name) : NULL;
}

int bmtop_fps_preflight(void) {
    fn_preflight preflight = (fn_preflight)cg_symbol("CGPreflightScreenCaptureAccess");
    return preflight != NULL && preflight() ? 1 : 0;
}

int bmtop_fps_start(void) {
    if (g_fps_stream != NULL) return 0; /* 幂等 */
    if (!bmtop_fps_preflight()) return -1;
    fn_create create = (fn_create)cg_symbol("CGDisplayStreamCreateWithDispatchQueue");
    fn_start start = (fn_start)cg_symbol("CGDisplayStreamStart");
    fn_drop_count drop_count = (fn_drop_count)cg_symbol("CGDisplayStreamUpdateGetDropCount");
    fn_main_display main_display = (fn_main_display)cg_symbol("CGMainDisplayID");
    CFStringRef *min_frame_key = (CFStringRef *)cg_symbol("kCGDisplayStreamMinimumFrameTime");
    CFStringRef *cursor_key = (CFStringRef *)cg_symbol("kCGDisplayStreamShowCursor");
    CFStringRef *depth_key = (CFStringRef *)cg_symbol("kCGDisplayStreamQueueDepth");
    if (!create || !start || !drop_count || !main_display || !min_frame_key || !cursor_key ||
        !depth_key) {
        return -2;
    }

    /* 帧回调上限 1/120s：读数只按实测窗口除法，封顶不影响正确性，省唤醒。 */
    double min_frame = 1.0 / 120.0;
    CFNumberRef min_frame_ref =
        CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &min_frame);
    int32_t depth = 1;
    CFNumberRef depth_ref = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &depth);
    const void *keys[] = {*min_frame_key, *cursor_key, *depth_key};
    const void *values[] = {min_frame_ref, kCFBooleanFalse, depth_ref};
    CFDictionaryRef properties =
        CFDictionaryCreate(kCFAllocatorDefault, keys, values, 3,
                           &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFRelease(min_frame_ref);
    CFRelease(depth_ref);

    if (g_fps_queue == NULL) {
        g_fps_queue = dispatch_queue_create("bmtop.displayfps", DISPATCH_QUEUE_SERIAL);
    }
    g_fps_stream = create(main_display(), 16, 16, 'BGRA', properties, g_fps_queue,
                          ^(int32_t status, uint64_t display_time, void *surface, void *update) {
                            (void)display_time;
                            (void)surface;
                            if (status != 0) return; /* 只数 FrameComplete */
                            uint64_t frames = 1;
                            if (update != NULL) frames += drop_count(update);
                            atomic_fetch_add(&g_fps_frames, frames);
                          });
    CFRelease(properties);
    if (g_fps_stream == NULL) return -2;
    if (start(g_fps_stream) != 0) {
        CFRelease((CFTypeRef)g_fps_stream);
        g_fps_stream = NULL;
        return -2;
    }
    atomic_store(&g_fps_frames, 0);
    g_fps_last_read = mach_absolute_time();
    return 0;
}

void bmtop_fps_stop(void) {
    if (g_fps_stream == NULL) return;
    fn_stop stop = (fn_stop)cg_symbol("CGDisplayStreamStop");
    if (stop != NULL) stop(g_fps_stream);
    CFRelease((CFTypeRef)g_fps_stream);
    g_fps_stream = NULL;
}

int bmtop_fps_read(int32_t *fps, double *frame_interval_ms) {
    if (fps == NULL || frame_interval_ms == NULL) return -1;
    *fps = 0;
    *frame_interval_ms = 0.0;
    if (g_fps_stream == NULL) return -1;
    static mach_timebase_info_data_t timebase = {0, 0};
    if (timebase.denom == 0) mach_timebase_info(&timebase);
    uint64_t now = mach_absolute_time();
    uint64_t elapsed_ns = (now - g_fps_last_read) * timebase.numer / timebase.denom;
    double seconds = (double)elapsed_ns / 1e9;
    uint64_t frames = atomic_exchange(&g_fps_frames, 0);
    g_fps_last_read = now;
    if (seconds > 0.1 && frames > 0) {
        *fps = (int32_t)((double)frames / seconds + 0.5);
        *frame_interval_ms = seconds * 1000.0 / (double)frames;
    }
    return 0;
}
