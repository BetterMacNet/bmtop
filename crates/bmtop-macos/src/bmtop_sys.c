#include "bmtop_sys.h"
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <libproc.h>
#include <mach/mach.h>
#include <mach/mach_host.h>
#include <mach/vm_statistics.h>
#include <net/if.h>
#include <net/if_dl.h>
#include <net/if_var.h>
#include <net/route.h>
#include <stdlib.h>
#include <string.h>
#include <sys/proc_info.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/sysctl.h>
#include <unistd.h>

int bmtop_read_cpu_ticks(bmtop_cpu_ticks *out) {
    if (!out) return -1;
    host_cpu_load_info_data_t info;
    mach_msg_type_number_t count = HOST_CPU_LOAD_INFO_COUNT;
    if (host_statistics(mach_host_self(), HOST_CPU_LOAD_INFO, (host_info_t)&info, &count) != KERN_SUCCESS) return -1;
    out->user = info.cpu_ticks[CPU_STATE_USER];
    out->system = info.cpu_ticks[CPU_STATE_SYSTEM];
    out->idle = info.cpu_ticks[CPU_STATE_IDLE];
    out->nice = info.cpu_ticks[CPU_STATE_NICE];
    return 0;
}

/* 每逻辑核的累计 tick。探测调用（out=NULL 或 capacity=0）回报核数。 */
size_t bmtop_read_core_ticks(bmtop_cpu_ticks *out, size_t capacity) {
    natural_t core_count = 0;
    processor_info_array_t info = NULL;
    mach_msg_type_number_t info_count = 0;
    if (host_processor_info(mach_host_self(), PROCESSOR_CPU_LOAD_INFO, &core_count, &info, &info_count) != KERN_SUCCESS) return 0;
    size_t written = 0;
    if (out && capacity > 0) {
        processor_cpu_load_info_t loads = (processor_cpu_load_info_t)info;
        for (natural_t index = 0; index < core_count && written < capacity; index++) {
            out[written].user = loads[index].cpu_ticks[CPU_STATE_USER];
            out[written].system = loads[index].cpu_ticks[CPU_STATE_SYSTEM];
            out[written].idle = loads[index].cpu_ticks[CPU_STATE_IDLE];
            out[written].nice = loads[index].cpu_ticks[CPU_STATE_NICE];
            written++;
        }
    }
    vm_deallocate(mach_task_self(), (vm_address_t)info, info_count * sizeof(integer_t));
    return (out && capacity > 0) ? written : (size_t)core_count;
}

int bmtop_read_memory(bmtop_memory_raw *out) {
    if (!out) return -1;
    vm_statistics64_data_t info;
    mach_msg_type_number_t count = HOST_VM_INFO64_COUNT;
    if (host_statistics64(mach_host_self(), HOST_VM_INFO64, (host_info64_t)&info, &count) != KERN_SUCCESS) return -1;
    size_t total = 0;
    size_t total_size = sizeof(total);
    if (sysctlbyname("hw.memsize", &total, &total_size, NULL, 0) != 0) return -1;
    out->total_bytes = total;
    out->free_pages = info.free_count;
    out->active_pages = info.active_count;
    out->inactive_pages = info.inactive_count;
    out->wired_pages = info.wire_count;
    out->compressed_pages = info.compressor_page_count;
    out->purgeable_pages = info.purgeable_count;
    out->swapins = info.swapins;
    out->swapouts = info.swapouts;
    /* swap 已用/总量。读不到就保持 0，调用方按「无 swap 数据」处理。 */
    struct xsw_usage swap;
    size_t swap_size = sizeof(swap);
    if (sysctlbyname("vm.swapusage", &swap, &swap_size, NULL, 0) == 0) {
        out->swap_total_bytes = swap.xsu_total;
        out->swap_used_bytes = swap.xsu_used;
    } else {
        out->swap_total_bytes = 0;
        out->swap_used_bytes = 0;
    }
    out->page_size = vm_page_size;
    return 0;
}

size_t bmtop_read_processes(bmtop_process_raw *out, size_t capacity) {
    /* proc_listallpids 两种调用都返回「进程个数」，不是字节数：
       传 NULL 回报当前进程数（带一点余量），传缓冲区回报实际填入的个数。
       缓冲区大小参数才是以字节计。原来的代码把返回值当字节数除以
       sizeof(pid_t)，只枚举到四分之一的进程。 */
    int probe_count = proc_listallpids(NULL, 0);
    if (probe_count <= 0) return 0;
    /* 探测调用（out=NULL 或 capacity=0）只回报需要多大的缓冲区。
       以前这里会直接走进下面 written < capacity 的循环，capacity 为 0 时
       恒返回 0，调用方据此认定「没有进程」并整个退回 ps 兜底路径，
       线程数和进程状态因此永远取不到。 */
    if (!out || capacity == 0) return (size_t)probe_count;
    size_t slots = (size_t)probe_count + 64;
    pid_t *pids = calloc(slots, sizeof(pid_t));
    if (!pids) return 0;
    int filled = proc_listallpids(pids, (int)(slots * sizeof(pid_t)));
    if (filled <= 0) { free(pids); return 0; }
    size_t available = (size_t)filled;
    if (available > slots) available = slots;
    size_t written = 0;
    for (size_t index = 0; index < available && written < capacity; index++) {
        pid_t pid = pids[index];
        if (pid <= 0) continue;
        struct proc_bsdinfo bsd;
        struct proc_taskinfo task;
        memset(&bsd, 0, sizeof(bsd));
        memset(&task, 0, sizeof(task));
        if (proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &bsd, sizeof(bsd)) != sizeof(bsd)) continue;
        proc_pidinfo(pid, PROC_PIDTASKINFO, 0, &task, sizeof(task));
        bmtop_process_raw *item = &out[written++];
        memset(item, 0, sizeof(*item));
        item->pid = pid;
        item->parent_pid = (int32_t)bsd.pbi_ppid;
        item->uid = (uint32_t)bsd.pbi_uid;
        item->status = bsd.pbi_status;
        item->thread_count = task.pti_threadnum;
        /* pbi_status 是 BSD 的 proc 状态，线程在睡也常年是 SRUN，
           所以 ps 的 R/S 其实来自线程。用正在运行的线程数还原这个语义。 */
        item->running_threads = task.pti_numrunning;
        /* fd 计数（PROC_PIDLISTFDS）明显比其他 flavor 贵，从全进程热路径
           移到 bmtop_read_fd_count，只对选中的进程按需调用。 */
        item->resident_bytes = task.pti_resident_size;
        item->virtual_bytes = task.pti_virtual_size;
        item->user_ticks = task.pti_total_user;
        item->system_ticks = task.pti_total_system;
        item->start_seconds = bsd.pbi_start_tvsec;
        item->start_microseconds = bsd.pbi_start_tvusec;
        /* 能耗影响的原料：QoS 分档 CPU 时间 + 空闲唤醒 + 磁盘字节。
           固定要 V3 而不是 RUSAGE_INFO_CURRENT——CURRENT 随 SDK 漂移（当前是 v6），
           而 V3 自 10.9 起布局冻结，且已含这里要的全部字段。
           无权限 / 进程已退出时 rusage_ok 留 0，上层据此给 None 而不是 0。 */
        struct rusage_info_v3 usage;
        memset(&usage, 0, sizeof(usage));
        if (proc_pid_rusage(pid, RUSAGE_INFO_V3, (rusage_info_t *)&usage) == 0) {
            item->qos_ns[0] = usage.ri_cpu_time_qos_default;
            item->qos_ns[1] = usage.ri_cpu_time_qos_maintenance;
            item->qos_ns[2] = usage.ri_cpu_time_qos_background;
            item->qos_ns[3] = usage.ri_cpu_time_qos_utility;
            item->qos_ns[4] = usage.ri_cpu_time_qos_legacy;
            item->qos_ns[5] = usage.ri_cpu_time_qos_user_initiated;
            item->qos_ns[6] = usage.ri_cpu_time_qos_user_interactive;
            item->idle_wakeups = usage.ri_pkg_idle_wkups;
            item->interrupt_wakeups = usage.ri_interrupt_wkups;
            item->disk_read_bytes = usage.ri_diskio_bytesread;
            item->disk_written_bytes = usage.ri_diskio_byteswritten;
            item->rusage_ok = 1;
        }
        proc_name(pid, item->name, sizeof(item->name));
        proc_pidpath(pid, item->path, sizeof(item->path));
    }
    free(pids);
    return written;
}

/* 接口计数器改走 NET_RT_IFLIST2（if_data64，64 位字节计数）。
   之前用 getifaddrs 的 if_data，ifi_ibytes 是 32 位，高吞吐接口几分钟
   就回绕一次，回绕的样本会被速率层整个丢掉。
   探测调用（out=NULL 或 capacity=0）回报接口个数。 */
size_t bmtop_read_interfaces(bmtop_interface_raw *out, size_t capacity) {
    int mib[6] = {CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0};
    size_t length = 0;
    if (sysctl(mib, 6, NULL, &length, NULL, 0) != 0 || length == 0) return 0;
    char *buffer = malloc(length);
    if (!buffer) return 0;
    if (sysctl(mib, 6, buffer, &length, NULL, 0) != 0) { free(buffer); return 0; }
    size_t total = 0;
    size_t written = 0;
    char *end = buffer + length;
    for (char *cursor = buffer; cursor + sizeof(struct if_msghdr) <= end;) {
        struct if_msghdr *header = (struct if_msghdr *)cursor;
        if (header->ifm_msglen == 0) break;
        cursor += header->ifm_msglen;
        if (header->ifm_type != RTM_IFINFO2) continue;
        struct if_msghdr2 *info = (struct if_msghdr2 *)header;
        struct sockaddr_dl *link = (struct sockaddr_dl *)(info + 1);
        if (link->sdl_family != AF_LINK || link->sdl_nlen == 0) continue;
        total++;
        if (!out || capacity == 0 || written >= capacity) continue;
        bmtop_interface_raw *item = &out[written++];
        memset(item, 0, sizeof(*item));
        size_t name_length = link->sdl_nlen;
        if (name_length >= sizeof(item->name)) name_length = sizeof(item->name) - 1;
        memcpy(item->name, link->sdl_data, name_length);
        item->received_bytes = info->ifm_data.ifi_ibytes;
        item->sent_bytes = info->ifm_data.ifi_obytes;
    }
    free(buffer);
    return (!out || capacity == 0) ? total : written;
}

/* 选中进程的 fd 数。失败（进程已退出 / 无权限）返回 -1。 */
int32_t bmtop_read_fd_count(int32_t pid) {
    int bytes = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, NULL, 0);
    return bytes > 0 ? (int32_t)(bytes / (int)sizeof(struct proc_fdinfo)) : -1;
}

/* 选中进程的线程列表（PROC_PIDLISTTHREADS + PROC_PIDTHREADINFO）。
   只对选中的一个进程调用——全进程 × 全线程枚举正是性能设计里要避免的。
   探测调用（out=NULL 或 capacity=0）回报线程个数。 */
size_t bmtop_read_threads(int32_t pid, bmtop_thread_raw *out, size_t capacity) {
    /* PROC_PIDLISTTHREADS 的 NULL 探测恒返回 0（和 PROC_PIDLISTFDS 不同，
       又一个 flavor 各自为政的坑），所以用 taskinfo 的线程数来定缓冲区。 */
    struct proc_taskinfo task;
    memset(&task, 0, sizeof(task));
    if (proc_pidinfo(pid, PROC_PIDTASKINFO, 0, &task, sizeof(task)) != sizeof(task)) return 0;
    if (task.pti_threadnum <= 0) return 0;
    size_t count = (size_t)task.pti_threadnum;
    if (!out || capacity == 0) return count;
    size_t slots = count + 16;
    int bytes;
    uint64_t *ids = calloc(slots, sizeof(uint64_t));
    if (!ids) return 0;
    bytes = proc_pidinfo(pid, PROC_PIDLISTTHREADS, 0, ids, (int)(slots * sizeof(uint64_t)));
    if (bytes <= 0) { free(ids); return 0; }
    size_t available = (size_t)bytes / sizeof(uint64_t);
    if (available > slots) available = slots;
    size_t written = 0;
    for (size_t index = 0; index < available && written < capacity; index++) {
        struct proc_threadinfo info;
        memset(&info, 0, sizeof(info));
        if (proc_pidinfo(pid, PROC_PIDTHREADINFO, ids[index], &info, sizeof(info)) != sizeof(info)) continue;
        bmtop_thread_raw *item = &out[written++];
        memset(item, 0, sizeof(*item));
        item->thread_id = ids[index];
        item->run_state = info.pth_run_state;
        /* pth_cpu_usage 以 TH_USAGE_SCALE(1000) 为满量程。 */
        item->cpu_percent = (double)info.pth_cpu_usage / 10.0;
        size_t name_length = sizeof(item->name) - 1;
        if (sizeof(info.pth_name) < name_length) name_length = sizeof(info.pth_name);
        memcpy(item->name, info.pth_name, name_length);
    }
    free(ids);
    return written;
}

/* 选中进程的累计磁盘读写字节（rusage_info）。 */
int bmtop_read_process_io(int32_t pid, uint64_t *disk_read_bytes, uint64_t *disk_written_bytes) {
    if (!disk_read_bytes || !disk_written_bytes) return -1;
    rusage_info_current usage;
    if (proc_pid_rusage(pid, RUSAGE_INFO_CURRENT, (rusage_info_t *)&usage) != 0) return -1;
    *disk_read_bytes = usage.ri_diskio_bytesread;
    *disk_written_bytes = usage.ri_diskio_byteswritten;
    return 0;
}

static double number_for_key(CFDictionaryRef dictionary, const char *key) {
    CFStringRef name = CFStringCreateWithCString(kCFAllocatorDefault, key, kCFStringEncodingUTF8);
    if (!name) return -1.0;
    const void *value = CFDictionaryGetValue(dictionary, name);
    double result = -1.0;
    if (value && CFGetTypeID(value) == CFNumberGetTypeID()) CFNumberGetValue((CFNumberRef)value, kCFNumberDoubleType, &result);
    CFRelease(name);
    return result;
}

int bmtop_read_gpu(bmtop_gpu_raw *out) {
    if (!out) return -1;
    CFMutableDictionaryRef matching = IOServiceMatching("IOAccelerator");
    if (!matching) return -1;
    io_iterator_t iterator = IO_OBJECT_NULL;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iterator) != KERN_SUCCESS) return -1;
    double best = -1.0;
    io_service_t service;
    while ((service = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
        CFTypeRef property = IORegistryEntryCreateCFProperty(service, CFSTR("PerformanceStatistics"), kCFAllocatorDefault, 0);
        if (property && CFGetTypeID(property) == CFDictionaryGetTypeID()) {
            CFDictionaryRef stats = (CFDictionaryRef)property;
            const char *keys[] = {"Device Utilization %", "GPU Activity(%)", "GPU Utilization %", "Renderer Utilization %", "Tiler Utilization %"};
            for (size_t i = 0; i < sizeof(keys) / sizeof(keys[0]); i++) {
                double value = number_for_key(stats, keys[i]);
                if (value >= 0.0 && value <= 100.0 && value > best) best = value;
            }
        }
        if (property) CFRelease(property);
        IOObjectRelease(service);
    }
    IOObjectRelease(iterator);
    if (best < 0.0) return -1;
    out->utilization_percent = best;
    out->idle_percent = 100.0 - best;
    return 0;
}

/* ---- 系统级磁盘 I/O（IOKit 注册表，无特权）----
 * Apple Silicon 上真实统计挂在 AppleAPFSVolume 的 Statistics 字典；
 * APFS 遍历四计数器全零时才退回传统 IOBlockStorageDriver
 * （mactop 按 kr!=success 兜底是死代码：无匹配也返回 success）。 */
static int64_t stats_number(CFDictionaryRef dict, const char *key) {
    CFStringRef key_ref =
        CFStringCreateWithCString(kCFAllocatorDefault, key, kCFStringEncodingUTF8);
    CFNumberRef number = CFDictionaryGetValue(dict, key_ref);
    CFRelease(key_ref);
    int64_t value = 0;
    if (number != NULL && CFGetTypeID(number) == CFNumberGetTypeID()) {
        CFNumberGetValue(number, kCFNumberSInt64Type, &value);
    }
    return value;
}

static int64_t stats_number_or(CFDictionaryRef dict, const char *primary, const char *fallback) {
    int64_t value = stats_number(dict, primary);
    return value != 0 ? value : stats_number(dict, fallback);
}

static void sum_disk_class(const char *class_name, uint64_t *read_bytes, uint64_t *write_bytes,
                           uint64_t *read_ops, uint64_t *write_ops) {
    io_iterator_t iterator;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching(class_name),
                                     &iterator) != kIOReturnSuccess) {
        return;
    }
    io_object_t entry;
    while ((entry = IOIteratorNext(iterator)) != 0) {
        CFMutableDictionaryRef props = NULL;
        if (IORegistryEntryCreateCFProperties(entry, &props, kCFAllocatorDefault, 0) ==
            kIOReturnSuccess) {
            CFDictionaryRef stats = CFDictionaryGetValue(props, CFSTR("Statistics"));
            if (stats != NULL && CFGetTypeID(stats) == CFDictionaryGetTypeID()) {
                *read_bytes +=
                    (uint64_t)stats_number_or(stats, "Bytes read from block device", "Bytes (Read)");
                *write_bytes += (uint64_t)stats_number_or(stats, "Bytes written to block device",
                                                          "Bytes (Write)");
                *read_ops += (uint64_t)stats_number_or(
                    stats, "Read requests sent to block device", "Operations (Read)");
                *write_ops += (uint64_t)stats_number_or(
                    stats, "Write requests sent to block device", "Operations (Write)");
            }
            CFRelease(props);
        }
        IOObjectRelease(entry);
    }
    IOObjectRelease(iterator);
}

int bmtop_read_disk_io(uint64_t *read_bytes, uint64_t *write_bytes, uint64_t *read_ops,
                      uint64_t *write_ops) {
    if (!read_bytes || !write_bytes || !read_ops || !write_ops) return -1;
    *read_bytes = *write_bytes = *read_ops = *write_ops = 0;
    sum_disk_class("AppleAPFSVolume", read_bytes, write_bytes, read_ops, write_ops);
    if (*read_bytes == 0 && *write_bytes == 0 && *read_ops == 0 && *write_ops == 0) {
        sum_disk_class("IOBlockStorageDriver", read_bytes, write_bytes, read_ops, write_ops);
    }
    return (*read_bytes || *write_bytes || *read_ops || *write_ops) ? 0 : -1;
}

/* ---- 每进程 GPU 时间（AGXDeviceUserClient 是 AGXAccelerator 的子节点）---- */
static uint64_t sum_app_usage(CFDictionaryRef props) {
    CFArrayRef usage = CFDictionaryGetValue(props, CFSTR("AppUsage"));
    if (usage == NULL || CFGetTypeID(usage) != CFArrayGetTypeID()) return 0;
    uint64_t total = 0;
    CFIndex count = CFArrayGetCount(usage);
    for (CFIndex i = 0; i < count; i++) {
        CFDictionaryRef record = CFArrayGetValueAtIndex(usage, i);
        if (record == NULL || CFGetTypeID(record) != CFDictionaryGetTypeID()) continue;
        CFNumberRef time_ref = CFDictionaryGetValue(record, CFSTR("accumulatedGPUTime"));
        if (time_ref == NULL || CFGetTypeID(time_ref) != CFNumberGetTypeID()) continue;
        int64_t value = 0;
        if (CFNumberGetValue(time_ref, kCFNumberSInt64Type, &value) && value > 0) {
            total += (uint64_t)value;
        }
    }
    return total;
}

size_t bmtop_read_gpu_process_times(bmtop_gpu_time_raw *out, size_t capacity) {
    if (!out || capacity == 0) return 0;
    io_service_t accelerator =
        IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("AGXAccelerator"));
    if (accelerator == 0) return 0;
    io_iterator_t children;
    if (IORegistryEntryGetChildIterator(accelerator, kIOServicePlane, &children) !=
        kIOReturnSuccess) {
        IOObjectRelease(accelerator);
        return 0;
    }
    size_t written = 0;
    io_object_t child;
    while ((child = IOIteratorNext(children)) != 0) {
        io_name_t class_name;
        IOObjectGetClass(child, class_name);
        if (strncmp(class_name, "AGXDeviceUserClient", 19) != 0) {
            IOObjectRelease(child);
            continue;
        }
        CFMutableDictionaryRef props = NULL;
        if (IORegistryEntryCreateCFProperties(child, &props, kCFAllocatorDefault, 0) ==
            kIOReturnSuccess) {
            CFStringRef creator = CFDictionaryGetValue(props, CFSTR("IOUserClientCreator"));
            char buffer[256] = {0};
            int pid = -1;
            if (creator != NULL &&
                CFStringGetCString(creator, buffer, sizeof(buffer), kCFStringEncodingUTF8)) {
                sscanf(buffer, "pid %d,", &pid);
            }
            uint64_t gpu_time = pid > 0 ? sum_app_usage(props) : 0;
            if (pid > 0 && gpu_time > 0) {
                /* 同一进程可能有多个 user client，就地合并 */
                size_t slot = written;
                for (size_t i = 0; i < written; i++) {
                    if (out[i].pid == pid) {
                        slot = i;
                        break;
                    }
                }
                if (slot < written) {
                    out[slot].gpu_time_ns += gpu_time;
                } else if (written < capacity) {
                    out[written].pid = pid;
                    out[written].gpu_time_ns = gpu_time;
                    written++;
                }
            }
            CFRelease(props);
        }
        IOObjectRelease(child);
    }
    IOObjectRelease(children);
    IOObjectRelease(accelerator);
    return written;
}
