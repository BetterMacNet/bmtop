/* Apple Silicon SoC 采集实现。逻辑移植自 mactop internal/app/ioreport.m + smc.c（MIT）。
 * 私有依赖：libIOReport（dyld shared cache，SDK 提供 .tbd 桩）。
 * 静态状态非线程安全：调用方保证单线程使用（见 soc.rs）。 */
#include "bmtop_soc.h"

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <mach/mach_time.h>
#include <IOKit/ps/IOPSKeys.h>
#include <IOKit/ps/IOPowerSources.h>
#include <notify.h>
#include <stdio.h>
#include <string.h>
#include <sys/sysctl.h>

/* ---- IOReport 私有 API ---- */
typedef struct IOReportSubscription *IOReportSubscriptionRef;
extern CFDictionaryRef IOReportCopyChannelsInGroup(CFStringRef group, CFStringRef subgroup,
                                                   uint64_t a, uint64_t b, uint64_t c);
extern void IOReportMergeChannels(CFDictionaryRef a, CFDictionaryRef b, CFTypeRef unused);
extern IOReportSubscriptionRef IOReportCreateSubscription(void *a, CFMutableDictionaryRef channels,
                                                          CFMutableDictionaryRef *out, uint64_t d,
                                                          CFTypeRef e);
extern CFDictionaryRef IOReportCreateSamples(IOReportSubscriptionRef sub,
                                             CFMutableDictionaryRef channels, CFTypeRef unused);
extern CFDictionaryRef IOReportCreateSamplesDelta(CFDictionaryRef a, CFDictionaryRef b,
                                                  CFTypeRef unused);
extern int64_t IOReportSimpleGetIntegerValue(CFDictionaryRef item, int32_t idx);
extern CFStringRef IOReportChannelGetGroup(CFDictionaryRef item);
extern CFStringRef IOReportChannelGetSubGroup(CFDictionaryRef item);
extern CFStringRef IOReportChannelGetChannelName(CFDictionaryRef item);
extern CFStringRef IOReportChannelGetUnitLabel(CFDictionaryRef item);
extern int32_t IOReportStateGetCount(CFDictionaryRef item);
extern CFStringRef IOReportStateGetNameForIndex(CFDictionaryRef item, int32_t idx);
extern int64_t IOReportStateGetResidency(CFDictionaryRef item, int32_t idx);

/* IOReport 内部会 autorelease；纯 C 里用 objc 运行时的池 API 包住采样。 */
extern void *objc_autoreleasePoolPush(void);
extern void objc_autoreleasePoolPop(void *pool);

/* ---- SMC（只读子集，移植自 mactop smc.c）---- */
#define KERNEL_INDEX_SMC 2
#define SMC_CMD_READ_BYTES 5
#define SMC_CMD_READ_INDEX 8
#define SMC_CMD_READ_KEYINFO 9
#define SMC_TYPE_FLT 0x666c7420u /* 'flt ' */

typedef struct {
    unsigned int dataSize;
    unsigned int dataType;
    char dataAttributes;
} smc_key_info;

typedef struct {
    unsigned int key;
    char vers[6];
    char pLimitData[16];
    smc_key_info keyInfo;
    char result;
    char status;
    char data8;
    unsigned int data32;
    char bytes[32];
} smc_key_data;

static io_connect_t g_smc_conn = 0;

static kern_return_t smc_call(smc_key_data *in, smc_key_data *out) {
    size_t out_size = sizeof(smc_key_data);
    kern_return_t rc = IOConnectCallStructMethod(g_smc_conn, KERNEL_INDEX_SMC, in,
                                                 sizeof(smc_key_data), out, &out_size);
    if (rc != kIOReturnSuccess) return rc;
    return out->result == 0 ? kIOReturnSuccess : kIOReturnError;
}

static unsigned int smc_fourcc(const char *key) {
    return ((unsigned int)(unsigned char)key[0] << 24) |
           ((unsigned int)(unsigned char)key[1] << 16) |
           ((unsigned int)(unsigned char)key[2] << 8) | (unsigned int)(unsigned char)key[3];
}

static io_connect_t smc_open(void) {
    io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("AppleSMC"),
                                     &iterator) != kIOReturnSuccess) {
        return 0;
    }
    io_object_t device = IOIteratorNext(iterator);
    IOObjectRelease(iterator);
    if (device == 0) return 0;
    io_connect_t conn = 0;
    kern_return_t rc = IOServiceOpen(device, mach_task_self(), 0, &conn);
    IOObjectRelease(device);
    return rc == kIOReturnSuccess ? conn : 0;
}

static kern_return_t smc_read_key(const char *key, smc_key_data *val) {
    smc_key_data in, out;
    memset(&in, 0, sizeof(in));
    memset(&out, 0, sizeof(out));
    memset(val, 0, sizeof(*val));
    in.key = smc_fourcc(key);
    in.data8 = SMC_CMD_READ_KEYINFO;
    kern_return_t rc = smc_call(&in, &out);
    if (rc != kIOReturnSuccess) return rc;
    val->keyInfo = out.keyInfo;
    in.keyInfo.dataSize = out.keyInfo.dataSize;
    in.data8 = SMC_CMD_READ_BYTES;
    rc = smc_call(&in, &out);
    if (rc != kIOReturnSuccess) return rc;
    memcpy(val->bytes, out.bytes, sizeof(out.bytes));
    return kIOReturnSuccess;
}

static double smc_float_value(const char *key) {
    smc_key_data val;
    if (smc_read_key(key, &val) != kIOReturnSuccess) return 0.0;
    if (val.keyInfo.dataType == SMC_TYPE_FLT && val.keyInfo.dataSize >= sizeof(float)) {
        float f;
        memcpy(&f, val.bytes, sizeof(float));
        return (double)f;
    }
    if (val.keyInfo.dataType == smc_fourcc("ui8 ") && val.keyInfo.dataSize >= 1) {
        return (double)(unsigned char)val.bytes[0];
    }
    return 0.0;
}

static int smc_key_count(void) {
    smc_key_data val;
    if (smc_read_key("#KEY", &val) != kIOReturnSuccess) return 0;
    return ((unsigned char)val.bytes[0] << 24) | ((unsigned char)val.bytes[1] << 16) |
           ((unsigned char)val.bytes[2] << 8) | (unsigned char)val.bytes[3];
}

static kern_return_t smc_key_from_index(int index, char *out_key) {
    smc_key_data in, out;
    memset(&in, 0, sizeof(in));
    memset(&out, 0, sizeof(out));
    in.data8 = SMC_CMD_READ_INDEX;
    in.data32 = (unsigned int)index;
    kern_return_t rc = smc_call(&in, &out);
    if (rc != kIOReturnSuccess) return rc;
    out_key[0] = (char)((out.key >> 24) & 0xFF);
    out_key[1] = (char)((out.key >> 16) & 0xFF);
    out_key[2] = (char)((out.key >> 8) & 0xFF);
    out_key[3] = (char)(out.key & 0xFF);
    out_key[4] = '\0';
    return kIOReturnSuccess;
}

static kern_return_t smc_key_info_for(const char *key, smc_key_info *info) {
    smc_key_data in, out;
    memset(&in, 0, sizeof(in));
    memset(&out, 0, sizeof(out));
    in.key = smc_fourcc(key);
    in.data8 = SMC_CMD_READ_KEYINFO;
    kern_return_t rc = smc_call(&in, &out);
    if (rc != kIOReturnSuccess) return rc;
    *info = out.keyInfo;
    return kIOReturnSuccess;
}

/* ---- 静态状态 ---- */
#define MAX_FREQ_STATES 64
#define MAX_TEMP_KEYS 64

static IOReportSubscriptionRef g_subscription = NULL;
static CFMutableDictionaryRef g_channels = NULL;
static CFDictionaryRef g_prev_sample = NULL;
static uint64_t g_prev_time = 0;

static uint32_t g_ecpu_freqs[MAX_FREQ_STATES], g_pcpu_freqs[MAX_FREQ_STATES],
    g_scpu_freqs[MAX_FREQ_STATES], g_gpu_freqs[MAX_FREQ_STATES];
static int g_ecpu_freq_count = 0, g_pcpu_freq_count = 0, g_scpu_freq_count = 0,
           g_gpu_freq_count = 0;

/* 采样时重读数值，key 列表只枚举一次（开机后不变）。 */
static char g_cpu_temp_keys[MAX_TEMP_KEYS][5];
static char g_gpu_temp_keys[MAX_TEMP_KEYS][5];
static char g_soc_temp_keys[MAX_TEMP_KEYS][5];
static char g_all_temp_keys[BMTOP_SOC_MAX_TEMPS][5];
static int g_cpu_temp_key_count = 0, g_gpu_temp_key_count = 0, g_soc_temp_key_count = 0,
           g_all_temp_key_count = 0;

static int g_thermal_token = -1;

/* 低于 10°C 的硅温度是开机未初始化的假读数（mactop issue #70）。 */
static const float kSiliconMinTempC = 10.0f;
static const float kMaxSaneTempC = 200.0f;

/* ---- 频率表（IORegistry pmgr voltage-states*）---- */
static void parse_freq_data(CFDataRef data, uint32_t *out_freqs, int *out_count) {
    if (data == NULL) return;
    CFIndex len = CFDataGetLength(data);
    const uint8_t *bytes = CFDataGetBytePtr(data);
    int total = (int)(len / 8);
    *out_count = 0;
    for (int i = 0; i < total && *out_count < MAX_FREQ_STATES; i++) {
        uint32_t freq = 0;
        memcpy(&freq, bytes + (size_t)i * 8, 4);
        uint32_t mhz = 0;
        if (freq >= 100000000u) {
            mhz = freq / 1000000u; /* Hz（M1–M4） */
        } else if (freq >= 100000u) {
            mhz = freq / 1000u; /* kHz（M5+） */
        }
        if (mhz > 0) out_freqs[(*out_count)++] = mhz;
    }
}

static void load_table(CFDictionaryRef props, const char *name, uint32_t *freqs, int *count) {
    if (*count > 0) return;
    CFStringRef key =
        CFStringCreateWithCString(kCFAllocatorDefault, name, kCFStringEncodingUTF8);
    CFDataRef data = (CFDataRef)CFDictionaryGetValue(props, key);
    CFRelease(key);
    if (data != NULL) parse_freq_data(data, freqs, count);
}

static void load_frequency_tables(void) {
    io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("AppleARMIODevice"),
                                     &iterator) != kIOReturnSuccess) {
        return;
    }
    io_object_t entry;
    while ((entry = IOIteratorNext(iterator)) != 0) {
        io_name_t name;
        IORegistryEntryGetName(entry, name);
        if (strcmp(name, "pmgr") == 0) {
            CFMutableDictionaryRef props = NULL;
            if (IORegistryEntryCreateCFProperties(entry, &props, kCFAllocatorDefault, 0) ==
                kIOReturnSuccess) {
                load_table(props, "voltage-states1-sram", g_ecpu_freqs, &g_ecpu_freq_count);
                load_table(props, "voltage-states9-sram", g_ecpu_freqs, &g_ecpu_freq_count);
                load_table(props, "voltage-states5-sram", g_pcpu_freqs, &g_pcpu_freq_count);
                load_table(props, "voltage-states3-sram", g_scpu_freqs, &g_scpu_freq_count);
                /* GPU：9-sram 在 M5+ 是 E 核表，但目标机型 M1–M4 上是 GPU 表 */
                load_table(props, "voltage-states9", g_gpu_freqs, &g_gpu_freq_count);
                load_table(props, "voltage-states9-sram", g_gpu_freqs, &g_gpu_freq_count);
                CFRelease(props);
            }
        }
        IOObjectRelease(entry);
        if (g_ecpu_freq_count > 0 && g_pcpu_freq_count > 0 && g_gpu_freq_count > 0) break;
    }
    IOObjectRelease(iterator);
}

/* ---- SMC 温度 key 枚举 ---- */
static void load_smc_temp_keys(void) {
    if (!g_smc_conn || g_all_temp_key_count > 0) return;
    int total = smc_key_count();
    for (int i = 0; i < total; i++) {
        char key[5];
        if (smc_key_from_index(i, key) != kIOReturnSuccess) continue;
        if (key[0] != 'T') continue;
        smc_key_info info;
        if (smc_key_info_for(key, &info) != kIOReturnSuccess) continue;
        if (info.dataType != SMC_TYPE_FLT) continue;
        float val = (float)smc_float_value(key);
        if (val > kMaxSaneTempC) continue; /* 枚举期就剔除坏传感器 */
        /* CPU：Tp/Te/Tf 每核（Tf 在 M3 上是 P 核）加 TCM/TCD die 级聚合，
         * M3 Max 等机型没有每核 key、只有 die 级。 */
        if (key[1] == 'p' || key[1] == 'e' || key[1] == 'f' ||
            (key[1] == 'C' && (key[2] == 'M' || key[2] == 'D'))) {
            if (g_cpu_temp_key_count < MAX_TEMP_KEYS)
                memcpy(g_cpu_temp_keys[g_cpu_temp_key_count++], key, 5);
        } else if (key[1] == 'g' || (key[1] == 'R' && key[2] == 'D')) {
            /* GPU：Tg* 集群 + TRD*（GPU Render Die，M3 系列） */
            if (g_gpu_temp_key_count < MAX_TEMP_KEYS)
                memcpy(g_gpu_temp_keys[g_gpu_temp_key_count++], key, 5);
        } else if (strncmp(key, "TPD", 3) == 0) {
            if (g_soc_temp_key_count < MAX_TEMP_KEYS)
                memcpy(g_soc_temp_keys[g_soc_temp_key_count++], key, 5);
        }
        if (g_all_temp_key_count < BMTOP_SOC_MAX_TEMPS)
            memcpy(g_all_temp_keys[g_all_temp_key_count++], key, 5);
    }
}

static double average_smc_keys(char keys[][5], int count) {
    double sum = 0.0;
    int used = 0;
    for (int i = 0; i < count; i++) {
        float val = (float)smc_float_value(keys[i]);
        if (val > kSiliconMinTempC && val < kMaxSaneTempC) {
            sum += val;
            used++;
        }
    }
    return used > 0 ? sum / used : 0.0;
}

/* ---- AMC Stats 通道匹配（DRAM/ANE 字节计数器，mactop 同款分类）---- */
/* token 两侧必须有界：前界 ^/空格/-/_/(//，后界 $/空格/-/_/)///+。
 * '+' 是合法后界，所以 "RD+WR" 必须先于 "RD" 判断。 */
static int str_has_token(const char *str, const char *token) {
    size_t token_len = strlen(token);
    for (const char *hit = strstr(str, token); hit != NULL; hit = strstr(hit + 1, token)) {
        char before = hit == str ? '\0' : hit[-1];
        char after = hit[token_len];
        int before_ok = before == '\0' || before == ' ' || before == '-' || before == '_' ||
                        before == '(' || before == '/';
        int after_ok = after == '\0' || after == ' ' || after == '-' || after == '_' ||
                       after == ')' || after == '/' || after == '+';
        if (before_ok && after_ok) return 1;
    }
    return 0;
}

/* 0 = combined/无方向；1 = 读；2 = 写 */
static int amc_direction(const char *chn) {
    if (strstr(chn, "RD+WR") != NULL || str_has_token(chn, "RW")) return 0;
    if (str_has_token(chn, "RD")) return 1;
    if (str_has_token(chn, "WR")) return 2;
    return 0;
}

static int is_exact_dcs(const char *chn) {
    return strcmp(chn, "DCS") == 0 || strcmp(chn, "DCS RD") == 0 || strcmp(chn, "DCS WR") == 0;
}

static int is_partition_dcs(const char *chn) {
    return strncmp(chn, "DCS_", 4) == 0 ||
           (strncmp(chn, "DCS", 3) == 0 && chn[3] >= '0' && chn[3] <= '9');
}

static int is_client_dcs(const char *chn) {
    return strstr(chn, " DCS ") != NULL;
}

/* AMC 桶累加器 */
typedef struct {
    int64_t exact_rd, exact_wr, exact_comb;
    int64_t part_rd, part_wr, part_comb;
    int64_t client_rd, client_wr;
    int64_t req_rd, req_wr;
    int64_t ane_rd, ane_wr;
    int has_exact_dir, has_exact_comb, has_part_dir, has_part_comb, has_client, has_req,
        has_ane;
} amc_acc;

static void accumulate_amc(const char *chn, int64_t val, amc_acc *acc) {
    if (val < 0) return; /* kIOReportInvalidIntValue 等无效值 */
    int dir = amc_direction(chn);
    /* ANE 侧通道（非 DCS）不参与 DCS 互斥链 */
    if (strncmp(chn, "ANE", 3) == 0 && strstr(chn, "DCS") == NULL) {
        if (dir == 1) acc->ane_rd += val, acc->has_ane = 1;
        else if (dir == 2) acc->ane_wr += val, acc->has_ane = 1;
    }
    if (is_exact_dcs(chn)) {
        if (dir == 1) acc->exact_rd += val, acc->has_exact_dir = 1;
        else if (dir == 2) acc->exact_wr += val, acc->has_exact_dir = 1;
        else acc->exact_comb += val, acc->has_exact_comb = 1;
    } else if (is_partition_dcs(chn)) {
        if (dir == 1) acc->part_rd += val, acc->has_part_dir = 1;
        else if (dir == 2) acc->part_wr += val, acc->has_part_dir = 1;
        else acc->part_comb += val, acc->has_part_comb = 1;
    } else if (is_client_dcs(chn)) {
        if (dir == 1) acc->client_rd += val, acc->has_client = 1;
        else if (dir == 2) acc->client_wr += val, acc->has_client = 1;
        /* 客户端 combined 丢弃 */
    } else if (dir == 1) {
        acc->req_rd += val, acc->has_req = 1;
    } else if (dir == 2) {
        acc->req_wr += val, acc->has_req = 1;
    }
}

/* 优先级：精确聚合 > 分区 > 客户端；request 桶仅当完全无 DCS 源时兜底
 * （精确聚合≈物理 DRAM 流量；客户端/请求计数器在多 die 上会重复计数）。 */
static void resolve_amc(const amc_acc *acc, bmtop_soc_sample_raw *out) {
    int has_source = acc->has_exact_dir || acc->has_exact_comb || acc->has_part_dir ||
                     acc->has_part_comb || acc->has_client;
    if (acc->has_exact_dir) {
        out->dram_read_bytes = acc->exact_rd;
        out->dram_write_bytes = acc->exact_wr;
    } else if (acc->has_exact_comb) {
        out->dram_read_bytes = acc->exact_comb / 2;
        out->dram_write_bytes = acc->exact_comb - acc->exact_comb / 2;
    } else if (acc->has_part_dir) {
        out->dram_read_bytes = acc->part_rd;
        out->dram_write_bytes = acc->part_wr;
    } else if (acc->has_part_comb) {
        out->dram_read_bytes = acc->part_comb / 2;
        out->dram_write_bytes = acc->part_comb - acc->part_comb / 2;
    } else if (acc->has_client) {
        out->dram_read_bytes = acc->client_rd;
        out->dram_write_bytes = acc->client_wr;
    } else if (acc->has_req) {
        out->dram_read_bytes = acc->req_rd;
        out->dram_write_bytes = acc->req_wr;
    }
    (void)has_source;
    if (acc->has_ane) {
        out->ane_read_bytes = acc->ane_rd;
        out->ane_write_bytes = acc->ane_wr;
    }
}

/* ---- 能量单位换算（unit label mJ/uJ/nJ，默认 uJ）---- */
static double energy_to_watts(int64_t energy, CFStringRef unit_ref, double duration_ms) {
    if (duration_ms <= 0) duration_ms = 1;
    double rate = (double)energy / (duration_ms / 1000.0);
    if (unit_ref == NULL) return rate / 1e6;
    char unit[32] = {0};
    CFStringGetCString(unit_ref, unit, sizeof(unit), kCFStringEncodingUTF8);
    for (int i = 0; unit[i]; i++) {
        if (unit[i] == ' ') unit[i] = '\0';
    }
    if (strcmp(unit, "mJ") == 0) return rate / 1e3;
    if (strcmp(unit, "nJ") == 0) return rate / 1e9;
    return rate / 1e6;
}

static uint64_t mach_time_to_ns(uint64_t delta) {
    static mach_timebase_info_data_t tb = {0, 0};
    if (tb.denom == 0) mach_timebase_info(&tb);
    return tb.denom ? delta * tb.numer / tb.denom : delta;
}

/* ---- 集群 residency 累加器 ---- */
typedef struct {
    double active_max;
    int freq_max;
    int seen;
} cluster_acc;

static void accumulate_cluster(CFDictionaryRef item, int is_e, int is_p, int is_s,
                               cluster_acc *e, cluster_acc *p, cluster_acc *s) {
    int32_t state_count = IOReportStateGetCount(item);
    int64_t total_time = 0, active_time = 0;
    double weighted_freq = 0;
    for (int32_t i = 0; i < state_count; i++) {
        int64_t residency = IOReportStateGetResidency(item, i);
        CFStringRef state_ref = IOReportStateGetNameForIndex(item, i);
        total_time += residency;
        if (state_ref == NULL) continue;
        char state[64] = {0};
        CFStringGetCString(state_ref, state, sizeof(state), kCFStringEncodingUTF8);
        if (strcmp(state, "OFF") == 0 || strcmp(state, "IDLE") == 0) continue;
        active_time += residency;
        int freq = 0;
        int v_idx = -1;
        if (state[0] == 'V' && sscanf(state, "V%d", &v_idx) == 1 && v_idx >= 0) {
            if (is_e && v_idx < g_ecpu_freq_count) freq = (int)g_ecpu_freqs[v_idx];
            else if (is_p && v_idx < g_pcpu_freq_count) freq = (int)g_pcpu_freqs[v_idx];
            else if (is_s && v_idx < g_scpu_freq_count) freq = (int)g_scpu_freqs[v_idx];
        }
        if (freq == 0) {
            for (int c = 0; state[c]; c++) {
                if (state[c] >= '0' && state[c] <= '9') {
                    freq = atoi(&state[c]);
                    break;
                }
            }
        }
        if (freq > 0) weighted_freq += (double)freq * residency;
    }
    if (total_time <= 0) return;
    double active_percent = (double)active_time / (double)total_time * 100.0;
    int avg_freq = active_time > 0 ? (int)(weighted_freq / active_time) : 0;
    cluster_acc *target = is_e ? e : (is_p ? p : s);
    /* 多 die 同类集群取 max（mactop 同款） */
    if (active_percent > target->active_max) target->active_max = active_percent;
    if (avg_freq > target->freq_max) target->freq_max = avg_freq;
    target->seen = 1;
}

static void emit_cluster(bmtop_soc_sample_raw *out, const char *name, const cluster_acc *acc) {
    if (!acc->seen || out->cluster_count >= BMTOP_SOC_MAX_CLUSTERS) return;
    bmtop_soc_cluster *slot = &out->clusters[out->cluster_count++];
    snprintf(slot->name, sizeof(slot->name), "%s", name);
    slot->active_percent = acc->active_max;
    slot->freq_mhz = acc->freq_max;
}

/* ---- GPU residency ---- */
static void read_gpu_states(CFDictionaryRef item, bmtop_soc_sample_raw *out) {
    int32_t state_count = IOReportStateGetCount(item);
    int64_t total_time = 0, active_time = 0;
    double weighted_freq = 0;
    int active_idx = 0;
    for (int32_t i = 0; i < state_count; i++) {
        int64_t residency = IOReportStateGetResidency(item, i);
        CFStringRef state_ref = IOReportStateGetNameForIndex(item, i);
        total_time += residency;
        if (state_ref == NULL) continue;
        char state[32] = {0};
        CFStringGetCString(state_ref, state, sizeof(state), kCFStringEncodingUTF8);
        if (strcmp(state, "OFF") == 0 || strcmp(state, "IDLE") == 0 ||
            strcmp(state, "DOWN") == 0) {
            continue;
        }
        active_time += residency;
        if (active_idx < g_gpu_freq_count)
            weighted_freq += (double)g_gpu_freqs[active_idx] * residency;
        active_idx++;
    }
    if (total_time > 0) out->gpu_active_percent = (double)active_time / (double)total_time * 100.0;
    if (active_time > 0 && g_gpu_freq_count > 0) out->gpu_freq_mhz = weighted_freq / active_time;
}

/* ---- init / sample / cleanup ---- */
int bmtop_soc_init(void) {
    if (g_subscription != NULL) return 0;

    CFDictionaryRef energy = IOReportCopyChannelsInGroup(CFSTR("Energy Model"), NULL, 0, 0, 0);
    if (energy == NULL) return -1; /* Intel 或系统不支持 */

    const CFStringRef extra_groups[] = {CFSTR("GPU Stats"), CFSTR("CPU Stats"),
                                        CFSTR("Energy Counters"), CFSTR("AMC Stats")};
    for (size_t i = 0; i < sizeof(extra_groups) / sizeof(extra_groups[0]); i++) {
        CFDictionaryRef chan = IOReportCopyChannelsInGroup(extra_groups[i], NULL, 0, 0, 0);
        if (chan != NULL) {
            IOReportMergeChannels(energy, chan, NULL);
            CFRelease(chan);
        }
    }

    g_channels = CFDictionaryCreateMutableCopy(kCFAllocatorDefault,
                                               CFDictionaryGetCount(energy), energy);
    CFRelease(energy);
    if (g_channels == NULL) return -2;

    CFMutableDictionaryRef subsystem = NULL;
    g_subscription = IOReportCreateSubscription(NULL, g_channels, &subsystem, 0, NULL);
    if (subsystem != NULL) CFRelease(subsystem);
    if (g_subscription == NULL) {
        CFRelease(g_channels);
        g_channels = NULL;
        return -3;
    }

    load_frequency_tables();
    g_smc_conn = smc_open();
    load_smc_temp_keys();
    if (notify_register_check("com.apple.system.thermalpressurelevel", &g_thermal_token) !=
        NOTIFY_STATUS_OK) {
        g_thermal_token = -1;
    }
    return 0;
}

int bmtop_soc_smc_available(void) { return g_smc_conn != 0; }
int bmtop_soc_thermal_available(void) { return g_thermal_token >= 0; }

static void read_smc_extras(bmtop_soc_sample_raw *out) {
    if (!g_smc_conn) return;

    /* 整机输入功率（墙上功率），并非各域之和 */
    double system_watts = smc_float_value("PSTR");
    if (system_watts > 0.0) out->system_watts = system_watts;

    out->cpu_temp_c = average_smc_keys(g_cpu_temp_keys, g_cpu_temp_key_count);
    out->gpu_temp_c = average_smc_keys(g_gpu_temp_keys, g_gpu_temp_key_count);
    double soc = average_smc_keys(g_soc_temp_keys, g_soc_temp_key_count);
    if (soc <= 0.0) {
        soc = out->cpu_temp_c > out->gpu_temp_c ? out->cpu_temp_c : out->gpu_temp_c;
    }
    out->soc_temp_c = soc;

    for (int i = 0; i < g_all_temp_key_count && out->temp_count < BMTOP_SOC_MAX_TEMPS; i++) {
        const char *key = g_all_temp_keys[i];
        float val = (float)smc_float_value(key);
        /* 硅温度（CPU/GPU die 系 key）低于 10°C 是未初始化假读数（mactop 同款过滤） */
        char k1 = key[1];
        int is_silicon = (k1 == 'p' || k1 == 'e' || k1 == 'f' || k1 == 'c' || k1 == 'C' ||
                          k1 == 'g' || k1 == 'R');
        float min_temp = is_silicon ? kSiliconMinTempC : 0.0f;
        if (val <= min_temp || val >= kMaxSaneTempC) continue;
        bmtop_soc_temp *slot = &out->temps[out->temp_count++];
        memcpy(slot->key, key, 5);
        slot->celsius = val;
    }

    smc_key_data fnum;
    if (smc_read_key("FNum", &fnum) == kIOReturnSuccess) {
        int fan_count = (unsigned char)fnum.bytes[0];
        if (fan_count > BMTOP_SOC_MAX_FANS) fan_count = BMTOP_SOC_MAX_FANS;
        for (int i = 0; i < fan_count; i++) {
            char key[5];
            bmtop_soc_fan *fan = &out->fans[out->fan_count];
            snprintf(key, sizeof(key), "F%dAc", i);
            fan->actual_rpm = (uint32_t)smc_float_value(key);
            snprintf(key, sizeof(key), "F%dMn", i);
            fan->min_rpm = (uint32_t)smc_float_value(key);
            snprintf(key, sizeof(key), "F%dMx", i);
            fan->max_rpm = (uint32_t)smc_float_value(key);
            snprintf(key, sizeof(key), "F%dTg", i);
            fan->target_rpm = (uint32_t)smc_float_value(key);
            out->fan_count++;
        }
    }
}

int bmtop_soc_sample(bmtop_soc_sample_raw *out) {
    if (g_subscription == NULL || g_channels == NULL || out == NULL) return -1;

    memset(out, 0, sizeof(*out));
    out->cpu_watts = out->gpu_watts = out->ane_watts = out->dram_watts = -1.0;
    out->system_watts = -1.0;
    out->gpu_active_percent = out->gpu_freq_mhz = -1.0;
    out->thermal_level = -1;
    out->dram_read_bytes = out->dram_write_bytes = -1;
    out->ane_read_bytes = out->ane_write_bytes = -1;

    void *pool = objc_autoreleasePoolPush();
    CFDictionaryRef sample = IOReportCreateSamples(g_subscription, g_channels, NULL);
    /* 时间锚点放在采样调用返回之后：调用本身的开销会同时出现在两个锚点里而抵消 */
    uint64_t now = mach_absolute_time();
    if (sample == NULL) {
        objc_autoreleasePoolPop(pool);
        return -1;
    }
    if (g_prev_sample == NULL) {
        g_prev_sample = sample;
        g_prev_time = now;
        objc_autoreleasePoolPop(pool);
        return 1; /* 预热完成，下次调用产出 delta */
    }

    CFDictionaryRef delta = IOReportCreateSamplesDelta(g_prev_sample, sample, NULL);
    CFRelease(g_prev_sample);
    uint64_t elapsed_ns = mach_time_to_ns(now - g_prev_time);
    g_prev_sample = sample;
    g_prev_time = now;
    if (delta == NULL) {
        objc_autoreleasePoolPop(pool);
        return -1;
    }
    out->elapsed_ns = elapsed_ns;
    double duration_ms = elapsed_ns > 0 ? (double)elapsed_ns / 1e6 : 1.0;

    /* 能量分桶：typed vs total 分开累计、循环后择优，防止双重计数 */
    double cpu_typed = 0, cpu_total = 0, gpu_named = 0, gpu_alias = 0, gpu_sram = 0;
    double ane_named = 0, ane_block = 0, dram_named = 0, dram_block = 0;
    int saw_cpu = 0, saw_gpu = 0, saw_ane = 0, saw_dram = 0;
    cluster_acc e_acc = {0}, p_acc = {0}, s_acc = {0};
    amc_acc amc = {0};

    CFArrayRef channels = CFDictionaryGetValue(delta, CFSTR("IOReportChannels"));
    CFIndex count = channels != NULL ? CFArrayGetCount(channels) : 0;
    for (CFIndex i = 0; i < count; i++) {
        CFDictionaryRef item = (CFDictionaryRef)CFArrayGetValueAtIndex(channels, i);
        if (item == NULL) continue;
        CFStringRef group_ref = IOReportChannelGetGroup(item);
        CFStringRef channel_ref = IOReportChannelGetChannelName(item);
        if (group_ref == NULL || channel_ref == NULL) continue;
        char grp[64] = {0}, chn[256] = {0};
        CFStringGetCString(group_ref, grp, sizeof(grp), kCFStringEncodingUTF8);
        CFStringGetCString(channel_ref, chn, sizeof(chn), kCFStringEncodingUTF8);

        if (strcmp(grp, "Energy Model") == 0 || strcmp(grp, "Energy Counters") == 0) {
            CFStringRef unit_ref = IOReportChannelGetUnitLabel(item);
            int64_t val = IOReportSimpleGetIntegerValue(item, 0);
            double watts = energy_to_watts(val, unit_ref, duration_ms);
            int typed_cpu = strstr(chn, "ECPU Energy") != NULL ||
                            strstr(chn, "PCPU Energy") != NULL ||
                            strstr(chn, "MCPU Energy") != NULL ||
                            strstr(chn, "eCPUs Energy") != NULL ||
                            strstr(chn, "pCPUs Energy") != NULL ||
                            strstr(chn, "mCPUs Energy") != NULL;
            if (typed_cpu) {
                cpu_typed += watts;
                saw_cpu = 1;
            } else if (strstr(chn, "CPU Energy") != NULL) {
                cpu_total += watts;
                saw_cpu = 1;
            } else if (strcmp(chn, "GPU Energy") == 0) {
                gpu_named += watts;
                saw_gpu = 1;
            } else if (strcmp(chn, "GPU") == 0) {
                gpu_alias += watts;
                saw_gpu = 1;
            } else if (strncmp(chn, "GPU SRAM", 8) == 0) {
                gpu_sram += watts; /* 并入 GPU（mactop 单列，这里为简化合并） */
                saw_gpu = 1;
            } else if (strstr(chn, "ANE") != NULL || strstr(chn, "NPU") != NULL ||
                       strstr(chn, "Neural") != NULL || strstr(chn, "ane") != NULL) {
                if (strstr(chn, "Energy") != NULL) ane_named += watts;
                else ane_block += watts;
                saw_ane = 1;
            } else if (strncmp(chn, "DRAM", 4) == 0) {
                if (strstr(chn, "Energy") != NULL) dram_named += watts;
                else dram_block += watts;
                saw_dram = 1;
            }
        } else if (strcmp(grp, "GPU Stats") == 0) {
            CFStringRef sub_ref = IOReportChannelGetSubGroup(item);
            if (sub_ref == NULL) continue;
            char sub[64] = {0};
            CFStringGetCString(sub_ref, sub, sizeof(sub), kCFStringEncodingUTF8);
            if (strcmp(sub, "GPU Performance States") == 0 && strcmp(chn, "GPUPH") == 0) {
                read_gpu_states(item, out);
            }
        } else if (strcmp(grp, "AMC Stats") == 0) {
            accumulate_amc(chn, IOReportSimpleGetIntegerValue(item, 0), &amc);
        } else if (strcmp(grp, "CPU Stats") == 0) {
            CFStringRef sub_ref = IOReportChannelGetSubGroup(item);
            if (sub_ref == NULL) continue;
            char sub[64] = {0};
            CFStringGetCString(sub_ref, sub, sizeof(sub), kCFStringEncodingUTF8);
            if (strcmp(sub, "CPU Complex Performance States") != 0) continue;
            /* MCPU0/MCPU1（M5+）内含 "CPU0"/"CPU1"，须先排除再做 legacy 匹配 */
            int is_m = strstr(chn, "MCPU") != NULL;
            int is_s = strstr(chn, "SCPU") != NULL;
            int is_e = strstr(chn, "ECPU") != NULL || (!is_m && strcmp(chn, "CPU0") == 0);
            int is_p = strstr(chn, "PCPU") != NULL || (!is_m && strcmp(chn, "CPU1") == 0);
            if (is_e || is_p || is_s) {
                accumulate_cluster(item, is_e, is_p, is_s, &e_acc, &p_acc, &s_acc);
            }
        }
    }
    CFRelease(delta);

    if (saw_cpu) out->cpu_watts = cpu_total > 0 ? cpu_total : cpu_typed;
    if (saw_gpu) out->gpu_watts = (gpu_named > 0 ? gpu_named : gpu_alias) + gpu_sram;
    if (saw_ane) out->ane_watts = ane_named > 0 ? ane_named : ane_block;
    if (saw_dram) out->dram_watts = dram_named > 0 ? dram_named : dram_block;
    emit_cluster(out, "E", &e_acc);
    emit_cluster(out, "P", &p_acc);
    emit_cluster(out, "S", &s_acc);
    resolve_amc(&amc, out);

    read_smc_extras(out);

    if (g_thermal_token >= 0) {
        uint64_t state = 0;
        if (notify_get_state(g_thermal_token, &state) == NOTIFY_STATUS_OK && state <= 4) {
            out->thermal_level = (int32_t)state;
        }
    }

    objc_autoreleasePoolPop(pool);
    return 0;
}

void bmtop_soc_cleanup(void) {
    if (g_prev_sample != NULL) {
        CFRelease(g_prev_sample);
        g_prev_sample = NULL;
    }
    if (g_subscription != NULL) {
        CFRelease(g_subscription);
        g_subscription = NULL;
    }
    if (g_channels != NULL) {
        CFRelease(g_channels);
        g_channels = NULL;
    }
    if (g_smc_conn != 0) {
        IOServiceClose(g_smc_conn);
        g_smc_conn = 0;
    }
    if (g_thermal_token >= 0) {
        notify_cancel(g_thermal_token);
        g_thermal_token = -1;
    }
}

/* ---- 电池（IOPowerSources，无特权）---- */
int bmtop_soc_read_battery(int32_t *percent, int32_t *charging, int32_t *on_ac) {
    if (percent == NULL || charging == NULL || on_ac == NULL) return 0;
    *percent = -1;
    *charging = 0;
    *on_ac = 0;
    CFTypeRef info = IOPSCopyPowerSourcesInfo();
    if (info == NULL) return 0;
    CFArrayRef list = IOPSCopyPowerSourcesList(info);
    if (list == NULL) {
        CFRelease(info);
        return 0;
    }
    int found = 0;
    CFIndex count = CFArrayGetCount(list);
    for (CFIndex i = 0; i < count && !found; i++) {
        /* Get 规则：描述字典不归调用方所有，不释放 */
        CFDictionaryRef ps = IOPSGetPowerSourceDescription(info, CFArrayGetValueAtIndex(list, i));
        if (ps == NULL) continue;
        CFStringRef type = CFDictionaryGetValue(ps, CFSTR(kIOPSTypeKey));
        if (type == NULL ||
            CFStringCompare(type, CFSTR(kIOPSInternalBatteryType), 0) != kCFCompareEqualTo) {
            continue;
        }
        found = 1;
        int32_t current = 0, max = 0;
        CFNumberRef current_ref = CFDictionaryGetValue(ps, CFSTR(kIOPSCurrentCapacityKey));
        CFNumberRef max_ref = CFDictionaryGetValue(ps, CFSTR(kIOPSMaxCapacityKey));
        if (current_ref != NULL && max_ref != NULL &&
            CFNumberGetValue(current_ref, kCFNumberSInt32Type, &current) &&
            CFNumberGetValue(max_ref, kCFNumberSInt32Type, &max) && max > 0) {
            *percent = current * 100 / max;
        }
        CFBooleanRef charging_ref = CFDictionaryGetValue(ps, CFSTR(kIOPSIsChargingKey));
        if (charging_ref != NULL && CFBooleanGetValue(charging_ref)) *charging = 1;
        CFStringRef state = CFDictionaryGetValue(ps, CFSTR(kIOPSPowerSourceStateKey));
        if (state != NULL &&
            CFStringCompare(state, CFSTR(kIOPSACPowerValue), 0) == kCFCompareEqualTo) {
            *on_ac = 1;
        }
    }
    CFRelease(list);
    CFRelease(info);
    return found;
}

/* ---- 静态拓扑（与 IOReport 无关）---- */
static int sysctl_string(const char *name, char *out, size_t capacity) {
    size_t len = capacity;
    if (sysctlbyname(name, out, &len, NULL, 0) != 0) return -1;
    out[capacity - 1] = '\0';
    return 0;
}

static int sysctl_i32(const char *name, int32_t *out) {
    int32_t value = 0;
    size_t len = sizeof(value);
    if (sysctlbyname(name, &value, &len, NULL, 0) != 0) return -1;
    *out = value;
    return 0;
}

static int32_t gpu_core_count(void) {
    io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("AGXAccelerator"),
                                     &iterator) != kIOReturnSuccess) {
        return 0;
    }
    int32_t cores = 0;
    io_object_t entry;
    while ((entry = IOIteratorNext(iterator)) != 0) {
        CFTypeRef value = IORegistryEntryCreateCFProperty(entry, CFSTR("gpu-core-count"),
                                                          kCFAllocatorDefault, 0);
        if (value != NULL) {
            if (CFGetTypeID(value) == CFNumberGetTypeID()) {
                CFNumberGetValue((CFNumberRef)value, kCFNumberSInt32Type, &cores);
            }
            CFRelease(value);
        }
        IOObjectRelease(entry);
        if (cores > 0) break;
    }
    IOObjectRelease(iterator);
    return cores;
}

int bmtop_soc_read_topology(bmtop_soc_topology_raw *out) {
    if (out == NULL) return -1;
    memset(out, 0, sizeof(*out));
    if (sysctl_string("machdep.cpu.brand_string", out->brand, sizeof(out->brand)) != 0) {
        return -1;
    }
    int32_t levels = 0;
    if (sysctl_i32("hw.nperflevels", &levels) == 0) {
        for (int32_t i = 0; i < levels && i < 8; i++) {
            char name_key[48], cpu_key[48], level_name[32] = {0};
            int32_t cpus = 0;
            snprintf(name_key, sizeof(name_key), "hw.perflevel%d.name", i);
            snprintf(cpu_key, sizeof(cpu_key), "hw.perflevel%d.logicalcpu", i);
            if (sysctl_string(name_key, level_name, sizeof(level_name)) != 0) continue;
            if (sysctl_i32(cpu_key, &cpus) != 0) continue;
            if (strncmp(level_name, "Performance", 11) == 0) out->p_cores += cpus;
            else if (strncmp(level_name, "Efficiency", 10) == 0) out->e_cores += cpus;
        }
    }
    out->gpu_cores = gpu_core_count();
    load_frequency_tables(); /* 幂等；init 未跑过时这里补一次 */
    uint32_t max_mhz = 0;
    for (int i = 0; i < g_gpu_freq_count; i++) {
        if (g_gpu_freqs[i] > max_mhz) max_mhz = g_gpu_freqs[i];
    }
    out->gpu_max_freq_mhz = (int32_t)max_mhz;
    return 0;
}
