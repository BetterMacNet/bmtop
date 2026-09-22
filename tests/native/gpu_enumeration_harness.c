/* Synthetic IOKit/CF adapter for the REAL GPU enumeration functions.
 * This validates C bounds/error contracts, NOT Apple SDK/ABI/hardware support.
 * test_native_contract.py injects the unchanged production function bodies. */
#include <assert.h>
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <limits.h>

typedef unsigned io_service_t;
typedef unsigned io_object_t;
typedef unsigned io_iterator_t;
typedef char io_name_t[128];
typedef long CFIndex;
typedef struct { int kind; int64_t number; char text[128]; } Fake;
typedef Fake *CFDictionaryRef;
typedef Fake *CFMutableDictionaryRef;
typedef Fake *CFArrayRef;
typedef Fake *CFStringRef;
typedef Fake *CFNumberRef;
typedef struct { int32_t pid; uint64_t gpu_time_ns; } bmtop_gpu_time_raw;
#define CFSTR(s) (s)
#define kIOMainPortDefault 0
#define kIOServicePlane "IOService"
#define kIOReturnSuccess 0
#define KERN_SUCCESS 0
#define kCFAllocatorDefault NULL
#define kCFStringEncodingUTF8 0
#define kCFNumberSInt64Type 0

static unsigned clients, cursor;
static int absent, iterator_failure, class_failure, bad_creator;
static Fake properties = {1, 0, ""}, usage = {2, 0, ""};
static Fake creator = {3, 0, "pid 42, synthetic"};
static Fake record = {1, 0, ""}, time_value = {4, 7, ""};
static const char *IOServiceMatching(const char *name) { return name; }
static io_service_t IOServiceGetMatchingService(int port, const char *name) {
    (void)port; (void)name; return absent ? 0 : 1;
}
static int IORegistryEntryGetChildIterator(io_service_t service, const char *plane, io_iterator_t *iterator) {
    (void)service; (void)plane; *iterator = 2; cursor = 0; return iterator_failure ? -1 : 0;
}
static io_object_t IOIteratorNext(io_iterator_t iterator) {
    (void)iterator; return cursor < clients ? 100 + cursor++ : 0;
}
static void IOObjectRelease(io_object_t object) { (void)object; }
static int IOObjectGetClass(io_object_t object, io_name_t output) {
    (void)object; strcpy(output, "AGXDeviceUserClient"); return class_failure ? -1 : 0;
}
static int IORegistryEntryCreateCFProperties(io_object_t object, CFMutableDictionaryRef *out, void *allocator, int flags) {
    (void)object; (void)allocator; (void)flags; *out = &properties; return 0;
}
static void *CFDictionaryGetValue(CFDictionaryRef object, const char *key) {
    (void)object;
    if (!strcmp(key, "AppUsage")) return &usage;
    if (!strcmp(key, "IOUserClientCreator")) return bad_creator ? &usage : &creator;
    if (!strcmp(key, "accumulatedGPUTime")) return &time_value;
    return NULL;
}
static int CFGetTypeID(const Fake *object) { return object->kind; }
static int CFDictionaryGetTypeID(void) { return 1; }
static int CFArrayGetTypeID(void) { return 2; }
static int CFStringGetTypeID(void) { return 3; }
static int CFNumberGetTypeID(void) { return 4; }
static CFIndex CFArrayGetCount(CFArrayRef object) { (void)object; return 3; }
static void *CFArrayGetValueAtIndex(CFArrayRef object, CFIndex index) { (void)object; (void)index; return &record; }
static int CFNumberGetValue(CFNumberRef object, int type, int64_t *out) {
    (void)type; *out = object->number; return 1;
}
static int CFStringGetCString(CFStringRef object, char *out, size_t capacity, int encoding) {
    (void)encoding; return snprintf(out, capacity, "%s", object->text) >= 0;
}
static void CFRelease(const Fake *object) { (void)object; }

#include "production_gpu_contract.inc"

int main(void) {
    clients = 600;
    assert(bmtop_read_gpu_process_times(NULL, 0) == 600);
    struct { bmtop_gpu_time_raw rows[256]; uint64_t guard; } small = { .guard = 0xC0FFEEULL };
    assert(bmtop_read_gpu_process_times(small.rows, 256) == 600);
    assert(small.rows[255].pid == 42 && small.rows[255].gpu_time_ns == 21);
    assert(small.guard == 0xC0FFEEULL);
    bmtop_gpu_time_raw large[1024] = {0};
    assert(bmtop_read_gpu_process_times(large, 1024) == 600);
    assert(large[599].gpu_time_ns == 21 && large[600].gpu_time_ns == 0);
    time_value.number = INT64_MAX;
    assert(sum_app_usage(&properties) == UINT64_MAX);
    time_value.number = -1;
    assert(sum_app_usage(&properties) == 0);
    time_value.number = 7;
    absent = 1; assert(bmtop_read_gpu_process_times(NULL, 0) == SIZE_MAX); absent = 0;
    iterator_failure = 1; assert(bmtop_read_gpu_process_times(NULL, 0) == SIZE_MAX); iterator_failure = 0;
    class_failure = 1; assert(bmtop_read_gpu_process_times(NULL, 0) == 0); class_failure = 0;
    bad_creator = 1; assert(bmtop_read_gpu_process_times(NULL, 0) == 0); bad_creator = 0;
    clients = 0; assert(bmtop_read_gpu_process_times(NULL, 0) == 0);
    puts("PASS: production GPU C enumeration bounds, sizing, saturation, sentinels and malformed-property contracts (synthetic IOKit adapter)");
    return 0;
}
