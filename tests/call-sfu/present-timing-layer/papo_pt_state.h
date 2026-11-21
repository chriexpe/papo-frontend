/*
 * Internal state, output I/O buffer, and seqlock latch reader for the
 * proof-only VK_EXT_present_timing Vulkan instrumentation layer.
 */
#ifndef PAPO_PT_STATE_H
#define PAPO_PT_STATE_H

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#include <vulkan/vk_layer.h>
#include <vulkan/vulkan.h>

#define PAPO_PT_MAX_INSTANCES 8
#define PAPO_PT_MAX_PHYS_DEVS 32
#define PAPO_PT_MAX_DEVICES 16
#define PAPO_PT_MAX_SWAPCHAINS 16
#define PAPO_PT_RING_CAP 256
#define PAPO_PT_DRAIN_BATCH 64
#define PAPO_PT_OUT_BUF_CAP 8192
#define PAPO_PT_LATCH_SIZE 128
#define PAPO_PT_LATCH_MAGIC 0x5041504F50544C31ULL

typedef struct PapoPtTile {
    uint64_t tile_idx;
    uint64_t pts_ns;
} PapoPtTile;

typedef struct PapoPtLatch {
    uint64_t magic;
    uint64_t seq;
    uint32_t tile_count;
    uint32_t valid_mask;
    PapoPtTile tiles[4];
    uint8_t pad[40];
} PapoPtLatch;

_Static_assert(sizeof(PapoPtLatch) == PAPO_PT_LATCH_SIZE, "PapoPtLatch must be 128 bytes");

typedef struct InflightPresent {
    bool occupied;
    uint64_t present_id;
    uint64_t latch_seq;
    uint64_t submit_mono_ns;
    uint32_t tile_count;
    uint32_t valid_mask;
    uint64_t tile_pts[4];
} InflightPresent;

typedef struct SwapchainData {
    bool in_use;
    VkSwapchainKHR swapchain;
    VkDevice device;
    bool timing_enabled;
    VkPresentStageFlagsEXT supported_stages;
    VkPresentStageFlagsEXT requested_stages;
    VkTimeDomainKHR chosen_domain;
    uint64_t chosen_domain_id;
    uint64_t refresh_duration_ns;
    uint64_t refresh_interval_ns;
    uint64_t next_present_id;
    uint64_t total_presents;
    uint64_t total_completed;
    uint64_t total_presented;
    uint64_t total_discarded;
    uint32_t overflow;
    InflightPresent ring[PAPO_PT_RING_CAP];
} SwapchainData;

typedef struct InstanceData {
    bool in_use;
    VkInstance instance;
    PFN_vkGetInstanceProcAddr next_gipa;
    PFN_vkDestroyInstance next_destroy_instance;
    PFN_vkEnumeratePhysicalDevices next_enum_phys_devs;
    PFN_vkGetPhysicalDeviceFeatures2 next_get_phys_features2;
    PFN_vkGetPhysicalDeviceSurfaceCapabilities2KHR next_get_surf_caps2;
    PFN_vkEnumerateDeviceExtensionProperties next_enum_dev_exts;
} InstanceData;

typedef struct PhysDevMap {
    bool in_use;
    VkPhysicalDevice phys;
    InstanceData *inst;
} PhysDevMap;

typedef struct DeviceData {
    bool in_use;
    VkDevice device;
    VkPhysicalDevice phys;
    InstanceData *inst;
    PFN_vkGetDeviceProcAddr next_gdpa;
    PFN_vkDestroyDevice next_destroy_device;
    PFN_vkCreateSwapchainKHR next_create_swapchain;
    PFN_vkDestroySwapchainKHR next_destroy_swapchain;
    PFN_vkQueuePresentKHR next_queue_present;
    PFN_vkSetSwapchainPresentTimingQueueSizeEXT next_set_pt_queue_size;
    PFN_vkGetSwapchainTimingPropertiesEXT next_get_pt_props;
    PFN_vkGetSwapchainTimeDomainPropertiesEXT next_get_pt_domains;
    PFN_vkGetPastPresentationTimingEXT next_get_past_pt;
    PFN_vkGetCalibratedTimestampsKHR next_get_calibrated_timestamps;
    bool present_timing_ext_enabled;
    bool present_id2_ext_enabled;
} DeviceData;

static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t g_io_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t g_latch_lock = PTHREAD_MUTEX_INITIALIZER;

static InstanceData g_instances[PAPO_PT_MAX_INSTANCES];
static PhysDevMap g_phys_devs[PAPO_PT_MAX_PHYS_DEVS];
static DeviceData g_devices[PAPO_PT_MAX_DEVICES];
static SwapchainData g_swapchains[PAPO_PT_MAX_SWAPCHAINS];

static int g_out_fd = -1;
static char g_out_buf[PAPO_PT_OUT_BUF_CAP];
static size_t g_out_len = 0;

static int g_latch_fd = -1;
static const PapoPtLatch *g_latch = NULL;

static uint64_t monotonic_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static void out_flush_locked(void) {
    if (g_out_fd < 0 || g_out_len == 0) {
        g_out_len = 0;
        return;
    }
    size_t off = 0;
    while (off < g_out_len) {
        ssize_t n = write(g_out_fd, g_out_buf + off, g_out_len - off);
        if (n > 0) {
            off += (size_t)n;
        } else if (n < 0 && errno == EINTR) {
            continue;
        } else {
            break;
        }
    }
    g_out_len = 0;
}

static void out_write_locked(const char *data, size_t len) {
    if (g_out_fd < 0 || len == 0) {
        return;
    }
    if (g_out_len + len > sizeof(g_out_buf)) {
        out_flush_locked();
    }
    if (len >= sizeof(g_out_buf)) {
        size_t off = 0;
        while (off < len) {
            ssize_t n = write(g_out_fd, data + off, len - off);
            if (n > 0) {
                off += (size_t)n;
            } else if (n < 0 && errno == EINTR) {
                continue;
            } else {
                break;
            }
        }
        return;
    }
    memcpy(g_out_buf + g_out_len, data, len);
    g_out_len += len;
}

static void io_printf(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void io_printf(const char *fmt, ...) {
    char line[512];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(line, sizeof(line), fmt, ap);
    va_end(ap);
    if (n <= 0) {
        return;
    }
    size_t len = (size_t)n;
    if (len >= sizeof(line)) {
        len = sizeof(line) - 1;
    }
    pthread_mutex_lock(&g_io_lock);
    out_write_locked(line, len);
    pthread_mutex_unlock(&g_io_lock);
}

static void io_flush(void) {
    pthread_mutex_lock(&g_io_lock);
    out_flush_locked();
    pthread_mutex_unlock(&g_io_lock);
}

static void init_io(void) {
    pthread_mutex_lock(&g_io_lock);
    if (g_out_fd < 0) {
        const char *out_path = getenv("PAPO_PT_OUT_FILE");
        if (out_path && out_path[0] != '\0') {
            g_out_fd = open(out_path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
        }
    }
    pthread_mutex_unlock(&g_io_lock);

    pthread_mutex_lock(&g_latch_lock);
    if (g_latch == NULL) {
        const char *latch_path = getenv("PAPO_PT_LATCH_FILE");
        if (latch_path && latch_path[0] != '\0') {
            int fd = open(latch_path, O_RDONLY | O_CLOEXEC);
            if (fd >= 0) {
                struct stat st;
                if (fstat(fd, &st) == 0 && st.st_size >= PAPO_PT_LATCH_SIZE) {
                    void *mapped = mmap(NULL, PAPO_PT_LATCH_SIZE, PROT_READ, MAP_SHARED, fd, 0);
                    if (mapped != MAP_FAILED) {
                        g_latch_fd = fd;
                        g_latch = (const PapoPtLatch *)mapped;
                    } else {
                        close(fd);
                    }
                } else {
                    close(fd);
                }
            }
        }
    }
    pthread_mutex_unlock(&g_latch_lock);
}

static void close_io(void) {
    pthread_mutex_lock(&g_io_lock);
    out_flush_locked();
    if (g_out_fd >= 0) {
        close(g_out_fd);
        g_out_fd = -1;
    }
    pthread_mutex_unlock(&g_io_lock);

    pthread_mutex_lock(&g_latch_lock);
    if (g_latch != NULL) {
        munmap((void *)(uintptr_t)g_latch, PAPO_PT_LATCH_SIZE);
        g_latch = NULL;
    }
    if (g_latch_fd >= 0) {
        close(g_latch_fd);
        g_latch_fd = -1;
    }
    pthread_mutex_unlock(&g_latch_lock);
}

static void read_latch(PapoPtLatch *out) {
    memset(out, 0, sizeof(*out));
    pthread_mutex_lock(&g_latch_lock);
    const PapoPtLatch *src = g_latch;
    if (src != NULL) {
        for (int attempt = 0; attempt < 64; attempt++) {
            uint64_t s1 = __atomic_load_n(&src->seq, __ATOMIC_ACQUIRE);
            if ((s1 & 1ULL) != 0) {
                continue;
            }
            uint64_t magic = __atomic_load_n(&src->magic, __ATOMIC_RELAXED);
            uint32_t tile_count = __atomic_load_n(&src->tile_count, __ATOMIC_RELAXED);
            uint32_t valid_mask = __atomic_load_n(&src->valid_mask, __ATOMIC_RELAXED);
            PapoPtTile tiles[4];
            for (uint32_t i = 0; i < 4; i++) {
                tiles[i].tile_idx = __atomic_load_n(&src->tiles[i].tile_idx, __ATOMIC_RELAXED);
                tiles[i].pts_ns = __atomic_load_n(&src->tiles[i].pts_ns, __ATOMIC_RELAXED);
            }
            __atomic_thread_fence(__ATOMIC_ACQUIRE);
            uint64_t s2 = __atomic_load_n(&src->seq, __ATOMIC_ACQUIRE);
            if (s1 == s2 && magic == PAPO_PT_LATCH_MAGIC && tile_count <= 4) {
                out->magic = magic;
                out->seq = s1;
                out->tile_count = tile_count;
                out->valid_mask = valid_mask;
                for (uint32_t i = 0; i < 4; i++) {
                    out->tiles[i] = tiles[i];
                }
                break;
            }
        }
    }
    pthread_mutex_unlock(&g_latch_lock);
}

static InstanceData *find_instance_locked(VkInstance inst) {
    for (size_t i = 0; i < PAPO_PT_MAX_INSTANCES; i++) {
        if (g_instances[i].in_use && g_instances[i].instance == inst) {
            return &g_instances[i];
        }
    }
    return NULL;
}

static InstanceData *find_instance_by_phys_locked(VkPhysicalDevice phys) {
    for (size_t i = 0; i < PAPO_PT_MAX_PHYS_DEVS; i++) {
        if (g_phys_devs[i].in_use && g_phys_devs[i].phys == phys) {
            return g_phys_devs[i].inst;
        }
    }
    for (size_t i = 0; i < PAPO_PT_MAX_INSTANCES; i++) {
        if (g_instances[i].in_use) {
            return &g_instances[i];
        }
    }
    return NULL;
}

static DeviceData *find_device_locked(VkDevice dev) {
    for (size_t i = 0; i < PAPO_PT_MAX_DEVICES; i++) {
        if (g_devices[i].in_use && g_devices[i].device == dev) {
            return &g_devices[i];
        }
    }
    return NULL;
}

static SwapchainData *find_swapchain_locked(VkSwapchainKHR sc) {
    for (size_t i = 0; i < PAPO_PT_MAX_SWAPCHAINS; i++) {
        if (g_swapchains[i].in_use && g_swapchains[i].swapchain == sc) {
            return &g_swapchains[i];
        }
    }
    return NULL;
}

static bool has_ext(const char *const *exts, uint32_t count, const char *target) {
    for (uint32_t i = 0; i < count; i++) {
        if (exts[i] && strcmp(exts[i], target) == 0) {
            return true;
        }
    }
    return false;
}

static const void *pnext_find_stype(const void *pnext, VkStructureType stype) {
    const VkBaseInStructure *cur = (const VkBaseInStructure *)pnext;
    while (cur != NULL) {
        if (cur->sType == stype) {
            return cur;
        }
        cur = cur->pNext;
    }
    return NULL;
}

static bool dev_supports_ext(const VkExtensionProperties *props, uint32_t count, const char *name) {
    for (uint32_t i = 0; i < count; i++) {
        if (strcmp(props[i].extensionName, name) == 0) {
            return true;
        }
    }
    return false;
}

static const char *time_domain_name(VkTimeDomainKHR domain) {
    switch (domain) {
    case VK_TIME_DOMAIN_DEVICE_KHR:
        return "DEVICE";
    case VK_TIME_DOMAIN_CLOCK_MONOTONIC_KHR:
        return "CLOCK_MONOTONIC";
    case VK_TIME_DOMAIN_CLOCK_MONOTONIC_RAW_KHR:
        return "CLOCK_MONOTONIC_RAW";
    case VK_TIME_DOMAIN_QUERY_PERFORMANCE_COUNTER_KHR:
        return "QPC";
    case VK_TIME_DOMAIN_PRESENT_STAGE_LOCAL_EXT:
        return "PRESENT_STAGE_LOCAL";
    case VK_TIME_DOMAIN_SWAPCHAIN_LOCAL_EXT:
        return "SWAPCHAIN_LOCAL";
    default:
        return "UNKNOWN";
    }
}

#endif
