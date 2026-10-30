/* Synthetic IOKit/CF adapter for the real disk I/O collector.
 * This validates zero-versus-unavailable behavior, not Apple SDK or hardware support. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef unsigned io_object_t;
typedef unsigned io_iterator_t;
typedef struct FakeValue FakeValue;
typedef FakeValue *CFDictionaryRef;
typedef FakeValue *CFMutableDictionaryRef;
typedef FakeValue *CFNumberRef;
typedef const char *CFStringRef;

#define CFSTR(value) (value)
#define kIOMainPortDefault 0
#define kIOReturnSuccess 0
#define kCFAllocatorDefault NULL
#define kCFStringEncodingUTF8 0
#define kCFNumberSInt64Type 0
#define KIND_STATS 2
#define KIND_NUMBER 4

struct FakeValue {
    int kind;
    int64_t number;
    unsigned present;
    FakeValue *statistics;
    FakeValue *numbers[4];
};

static FakeValue apfs_stats, block_stats;
static FakeValue apfs_numbers[4], block_numbers[4];
static FakeValue apfs_properties, block_properties;
static const char *matching_class;
static int iterator_used;
static int no_apfs;
static int no_block;

static const char *IOServiceMatching(const char *class_name) { return class_name; }

static int IOServiceGetMatchingServices(int port, const char *matching, io_iterator_t *iterator) {
    (void)port;
    matching_class = matching;
    iterator_used = 0;
    *iterator = 1;
    if (!strcmp(matching, "AppleAPFSVolume") && no_apfs) return -1;
    if (!strcmp(matching, "IOBlockStorageDriver") && no_block) return -1;
    return kIOReturnSuccess;
}

static io_object_t IOIteratorNext(io_iterator_t iterator) {
    (void)iterator;
    return iterator_used++ == 0 ? 42 : 0;
}

static int IORegistryEntryCreateCFProperties(io_object_t entry, CFMutableDictionaryRef *out,
                                             void *allocator, int flags) {
    (void)entry;
    (void)allocator;
    (void)flags;
    *out = !strcmp(matching_class, "AppleAPFSVolume") ? &apfs_properties : &block_properties;
    return kIOReturnSuccess;
}

static void IOObjectRelease(io_object_t object) { (void)object; }
static CFStringRef CFStringCreateWithCString(void *allocator, const char *value, int encoding) {
    (void)allocator;
    (void)encoding;
    return value;
}
static void CFRelease(const void *value) { (void)value; }

static int CFGetTypeID(const FakeValue *value) { return value->kind; }
static int CFDictionaryGetTypeID(void) { return KIND_STATS; }
static int CFNumberGetTypeID(void) { return KIND_NUMBER; }

static int counter_index(const char *key) {
    if (!strcmp(key, "Bytes read from block device") || !strcmp(key, "Bytes (Read)")) return 0;
    if (!strcmp(key, "Bytes written to block device") || !strcmp(key, "Bytes (Write)")) return 1;
    if (!strcmp(key, "Read requests sent to block device") || !strcmp(key, "Operations (Read)")) return 2;
    if (!strcmp(key, "Write requests sent to block device") || !strcmp(key, "Operations (Write)")) return 3;
    return -1;
}

static void *CFDictionaryGetValue(CFDictionaryRef dictionary, CFStringRef key) {
    if (!strcmp(key, "Statistics")) return dictionary->statistics;
    int index = counter_index(key);
    if (index < 0 || !(dictionary->present & (1u << index))) return NULL;
    return dictionary->numbers[index];
}

static int CFNumberGetValue(CFNumberRef number, int type, int64_t *out) {
    (void)type;
    *out = number->number;
    return 1;
}

static void set_stats(FakeValue *stats, FakeValue *numbers, unsigned present,
                      int64_t read_bytes, int64_t write_bytes, int64_t read_ops,
                      int64_t write_ops) {
    int64_t values[] = {read_bytes, write_bytes, read_ops, write_ops};
    stats->kind = KIND_STATS;
    stats->present = present;
    for (int index = 0; index < 4; index++) {
        numbers[index].kind = KIND_NUMBER;
        numbers[index].number = values[index];
        stats->numbers[index] = &numbers[index];
    }
}

static void set_properties(FakeValue *properties, FakeValue *stats) {
    properties->kind = KIND_STATS;
    properties->statistics = stats;
    properties->present = 0;
}

#include "production_disk_io_contract.inc"

int main(void) {
    uint64_t counters[4];

    set_stats(&apfs_stats, apfs_numbers, 0xF, 0, 0, 0, 0);
    set_stats(&block_stats, block_numbers, 0xF, 10, 20, 30, 40);
    set_properties(&apfs_properties, &apfs_stats);
    set_properties(&block_properties, &block_stats);
    assert(bmtop_read_disk_io(&counters[0], &counters[1], &counters[2], &counters[3]) == 0);
    assert(counters[0] == 0 && counters[1] == 0 && counters[2] == 0 && counters[3] == 0);

    set_properties(&apfs_properties, NULL);
    assert(bmtop_read_disk_io(&counters[0], &counters[1], &counters[2], &counters[3]) == 0);
    assert(counters[0] == 10 && counters[1] == 20 && counters[2] == 30 && counters[3] == 40);

    apfs_properties.statistics = NULL;
    block_properties.statistics = NULL;
    set_properties(&apfs_properties, NULL);
    set_properties(&block_properties, NULL);
    no_apfs = 1;
    no_block = 1;
    assert(bmtop_read_disk_io(&counters[0], &counters[1], &counters[2], &counters[3]) == -1);
    puts("PASS: disk I/O preserves real zero counters and distinguishes unavailable sources");
    return 0;
}
