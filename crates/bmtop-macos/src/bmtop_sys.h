#include <stdint.h>
#include <stddef.h>

typedef struct { uint64_t user, system, idle, nice; } bmtop_cpu_ticks;
typedef struct { uint64_t total_bytes, free_pages, active_pages, inactive_pages, wired_pages, compressed_pages, purgeable_pages, swapins, swapouts, swap_total_bytes, swap_used_bytes, page_size; } bmtop_memory_raw;
/* qos_ns 的下标顺序与 bmtop_core 的 QOS_WEIGHT_INDEX 一一对应，改一边必须改另一边：
   0 default, 1 maintenance, 2 background, 3 utility, 4 legacy,
   5 user_initiated, 6 user_interactive。 */
#define BMTOP_QOS_BUCKETS 7
typedef struct { int32_t pid, parent_pid; uint32_t uid, status; int32_t thread_count, running_threads; uint64_t resident_bytes, virtual_bytes, user_ticks, system_ticks, start_seconds, start_microseconds; uint64_t qos_ns[BMTOP_QOS_BUCKETS]; uint64_t idle_wakeups, interrupt_wakeups, disk_read_bytes, disk_written_bytes; uint32_t rusage_ok; char name[64]; char path[1024]; } bmtop_process_raw;
typedef struct { char name[64]; uint64_t received_bytes, sent_bytes; } bmtop_interface_raw;
typedef struct { double utilization_percent, idle_percent; } bmtop_gpu_raw;
typedef struct { uint64_t thread_id; int32_t run_state; double cpu_percent; char name[64]; } bmtop_thread_raw;

int bmtop_read_cpu_ticks(bmtop_cpu_ticks *out);
size_t bmtop_read_core_ticks(bmtop_cpu_ticks *out, size_t capacity);
int bmtop_read_memory(bmtop_memory_raw *out);
size_t bmtop_read_processes(bmtop_process_raw *out, size_t capacity);
size_t bmtop_read_interfaces(bmtop_interface_raw *out, size_t capacity);
int bmtop_read_gpu(bmtop_gpu_raw *out);
int32_t bmtop_read_fd_count(int32_t pid);
int bmtop_read_process_io(int32_t pid, uint64_t *disk_read_bytes, uint64_t *disk_written_bytes);
size_t bmtop_read_threads(int32_t pid, bmtop_thread_raw *out, size_t capacity);
/* 系统级磁盘 I/O 累计计数（开机以来，全卷求和）。0 成功，-1 失败。 */
int bmtop_read_disk_io(uint64_t *read_bytes, uint64_t *write_bytes, uint64_t *read_ops, uint64_t *write_ops);
/* 每进程累计 GPU 时间（纳秒）。返回填入条数。 */
typedef struct { int32_t pid; uint64_t gpu_time_ns; } bmtop_gpu_time_raw;
size_t bmtop_read_gpu_process_times(bmtop_gpu_time_raw *out, size_t capacity);
