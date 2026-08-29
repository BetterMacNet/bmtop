/* 网络链路（Ethernet/Wi-Fi）、雷雳拓扑、屏幕 FPS。移植自 mactop（MIT）。
 * 全部无需 sudo；FPS 需要「屏幕录制」授权（只探测，绝不主动弹窗）。 */
#ifndef BMTOP_LINK_H
#define BMTOP_LINK_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
    char name[32];
    uint64_t speed_mbps; /* 0 = 断开或未知 */
    int32_t link_up;
} bmtop_eth_link;
size_t bmtop_read_ethernet_links(bmtop_eth_link *out, size_t capacity);

typedef struct {
    char name[32];
    char phy_mode[32];   /* "802.11ax" 等 */
    char generation[16]; /* "Wi-Fi 6" 等，PHY 未知时为空 */
    int32_t tx_rate_mbps;
    int32_t connected;
} bmtop_wifi_link;
/* 0 = 成功；-1 = 无 Wi-Fi 硬件或 CoreWLAN 不可用。 */
int bmtop_read_wifi_link(bmtop_wifi_link *out);

typedef struct {
    int64_t uid;
    int64_t parent_uid; /* depth>0 时为所属总线（depth==0 节点）的 UID；0 = 未解析 */
    int32_t depth;      /* 0 = 主机总线，>0 = 外接设备 */
    int32_t link_speed;    /* Supported Link Speed（能力档） */
    int32_t current_speed; /* Current Link Speed（协商档），0 = 未知 */
    char vendor[64];
    char device[128];
} bmtop_tb_switch;
size_t bmtop_read_tb_switches(bmtop_tb_switch *out, size_t capacity);

/* 屏幕 FPS（CGDisplayStream 经 dlopen；SDK 已标 unavailable 但符号仍在）。 */
int bmtop_fps_preflight(void); /* 1 = 已授权屏幕录制（只查不弹） */
int bmtop_fps_start(void);     /* 0 成功；-1 未授权；-2 符号缺失/创建失败；幂等 */
void bmtop_fps_stop(void);
/* 自上次调用以来的合成帧率；0 成功。窗口 <100ms 或无帧时 fps=0。 */
int bmtop_fps_read(int32_t *fps, double *frame_interval_ms);

#endif
