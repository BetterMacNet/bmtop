/* Apple Silicon SoC 采集（IOReport + SMC + notify），移植自 mactop（MIT）。
 * 全部只读、无需 sudo。Intel 或 IOReport 缺失时 init 返回负值，调用方降级。 */
#ifndef BMTOP_SOC_H
#define BMTOP_SOC_H

#include <stdint.h>

#define BMTOP_SOC_MAX_CLUSTERS 8
#define BMTOP_SOC_MAX_FANS 8
#define BMTOP_SOC_MAX_TEMPS 256

typedef struct {
    char name[8]; /* "E" / "P" / "S" */
    double active_percent;
    double freq_mhz;
} bmtop_soc_cluster;

typedef struct {
    uint32_t actual_rpm, min_rpm, max_rpm, target_rpm;
} bmtop_soc_fan;

typedef struct {
    char key[5]; /* SMC 4 字符 key + NUL */
    float celsius;
} bmtop_soc_temp;

typedef struct {
    int32_t cluster_count;
    bmtop_soc_cluster clusters[BMTOP_SOC_MAX_CLUSTERS];
    double cpu_watts, gpu_watts, ane_watts, dram_watts; /* <0 = 不可用 */
    double gpu_active_percent, gpu_freq_mhz;            /* <0 = 不可用 */
    double cpu_temp_c, gpu_temp_c, soc_temp_c;          /* <=0 = 不可用 */
    int32_t thermal_level;                              /* -1 = 不可用 */
    int32_t fan_count;
    bmtop_soc_fan fans[BMTOP_SOC_MAX_FANS];
    int32_t temp_count;
    bmtop_soc_temp temps[BMTOP_SOC_MAX_TEMPS];
    double system_watts;                                /* <0 = 不可用（SMC PSTR） */
    int64_t dram_read_bytes, dram_write_bytes;          /* <0 = 无 AMC 源 */
    int64_t ane_read_bytes, ane_write_bytes;            /* <0 = 无 AMC 源 */
    uint64_t elapsed_ns;
} bmtop_soc_sample_raw;

typedef struct {
    char brand[64];
    int32_t e_cores, p_cores, gpu_cores; /* 0 = 未知 */
    int32_t gpu_max_freq_mhz;            /* 0 = 未知（pmgr 频率表最大档） */
} bmtop_soc_topology_raw;

/* 0 成功；-1 无 Energy Model（Intel）；-2/-3 通道表或订阅失败。幂等。 */
int bmtop_soc_init(void);
/* 0 成功；1 仅预热（首次调用，尚无 delta）；-1 未初始化或采样失败。 */
int bmtop_soc_sample(bmtop_soc_sample_raw *out);
void bmtop_soc_cleanup(void);
/* 0 成功（字段尽力填充）；-1 完全失败。与 IOReport 无关，Intel 也可用。 */
int bmtop_soc_read_topology(bmtop_soc_topology_raw *out);
/* SMC 连接是否可用（诊断用，需先 init）。 */
int bmtop_soc_smc_available(void);
/* 热压力 notify 是否可用（诊断用，需先 init）。 */
int bmtop_soc_thermal_available(void);
/* 电池：返回 0 = 无内置电池；1 = 有。percent = -1 表示电量未知。 */
int bmtop_soc_read_battery(int32_t *percent, int32_t *charging, int32_t *on_ac);

#endif
