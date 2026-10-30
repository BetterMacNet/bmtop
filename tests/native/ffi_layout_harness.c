/* C-side ABI contract for the Rust repr(C) mirrors. */
#include <assert.h>
#include <stddef.h>
#include <stdio.h>

#include "bmtop_link.h"
#include "bmtop_soc.h"
#include "bmtop_sys.h"

#define ASSERT_SIZE(type, expected) \
    _Static_assert(sizeof(type) == (expected), "FFI size changed: " #type)
#define ASSERT_OFFSET(type, field, expected) \
    _Static_assert(offsetof(type, field) == (expected), "FFI offset changed: " #type "." #field)

ASSERT_SIZE(bmtop_cpu_ticks, 32);
ASSERT_OFFSET(bmtop_cpu_ticks, user, 0);
ASSERT_OFFSET(bmtop_cpu_ticks, nice, 24);

ASSERT_SIZE(bmtop_memory_raw, 96);
ASSERT_OFFSET(bmtop_memory_raw, swap_total_bytes, 72);
ASSERT_OFFSET(bmtop_memory_raw, page_size, 88);

ASSERT_SIZE(bmtop_process_raw, 1256);
ASSERT_OFFSET(bmtop_process_raw, qos_ns, 72);
ASSERT_OFFSET(bmtop_process_raw, rusage_ok, 160);
ASSERT_OFFSET(bmtop_process_raw, path, 228);

ASSERT_SIZE(bmtop_interface_raw, 80);
ASSERT_OFFSET(bmtop_interface_raw, received_bytes, 64);
ASSERT_SIZE(bmtop_gpu_raw, 16);
ASSERT_SIZE(bmtop_thread_raw, 88);
ASSERT_OFFSET(bmtop_thread_raw, cpu_percent, 16);
ASSERT_SIZE(bmtop_gpu_time_raw, 16);
ASSERT_OFFSET(bmtop_gpu_time_raw, gpu_time_ns, 8);

ASSERT_SIZE(bmtop_soc_cluster, 24);
ASSERT_SIZE(bmtop_soc_fan, 16);
ASSERT_SIZE(bmtop_soc_temp, 12);
ASSERT_OFFSET(bmtop_soc_temp, celsius, 8);
ASSERT_SIZE(bmtop_soc_sample_raw, 3536);
ASSERT_OFFSET(bmtop_soc_sample_raw, clusters, 8);
ASSERT_OFFSET(bmtop_soc_sample_raw, temps, 412);
ASSERT_OFFSET(bmtop_soc_sample_raw, elapsed_ns, 3528);
ASSERT_SIZE(bmtop_soc_topology_raw, 80);
ASSERT_OFFSET(bmtop_soc_topology_raw, p_cores, 68);

ASSERT_SIZE(bmtop_eth_link, 48);
ASSERT_OFFSET(bmtop_eth_link, speed_mbps, 32);
ASSERT_SIZE(bmtop_wifi_link, 88);
ASSERT_OFFSET(bmtop_wifi_link, tx_rate_mbps, 80);
ASSERT_SIZE(bmtop_tb_switch, 224);
ASSERT_OFFSET(bmtop_tb_switch, vendor, 28);
ASSERT_OFFSET(bmtop_tb_switch, device, 92);

int main(void) {
    puts("PASS: C/Rust FFI size and offset contract");
    return 0;
}
