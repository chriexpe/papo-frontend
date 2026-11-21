/*
 * Proof-only Vulkan explicit layer enabling VK_EXT_present_timing + VK_KHR_present_id2
 * without modifying Rune or wgpu-hal. Records compositor-verified presentation times
 * keyed by presentId and matched to a shared memory-mapped frame latch.
 */
#include "papo_pt_state.h"

typedef struct DrainEventRecord {
    uint64_t present_id;
    InflightPresent matched;
    uint64_t first_pixel_out_ns;
    uint64_t first_pixel_visible_ns;
    uint64_t queue_end_ns;
    uint32_t stages_mask;
    VkTimeDomainKHR time_domain;
    uint32_t report_complete;
    uint32_t presented;
} DrainEventRecord;

static void drain_swapchain_timings(
    VkDevice device,
    VkSwapchainKHR swapchain,
    PFN_vkGetPastPresentationTimingEXT get_past_pt
) {
    if (!get_past_pt || device == VK_NULL_HANDLE || swapchain == VK_NULL_HANDLE) {
        return;
    }
    VkPastPresentationTimingInfoEXT query = {
        .sType = VK_STRUCTURE_TYPE_PAST_PRESENTATION_TIMING_INFO_EXT,
        .pNext = NULL,
        .flags = VK_PAST_PRESENTATION_TIMING_ALLOW_OUT_OF_ORDER_RESULTS_BIT_EXT,
        .swapchain = swapchain,
    };
    VkPresentStageTimeEXT stage_storage[PAPO_PT_DRAIN_BATCH][4];
    VkPastPresentationTimingEXT timings[PAPO_PT_DRAIN_BATCH];
    DrainEventRecord records[PAPO_PT_DRAIN_BATCH];

    for (int iter = 0; iter < 16; iter++) {
        for (uint32_t i = 0; i < PAPO_PT_DRAIN_BATCH; i++) {
            timings[i].sType = VK_STRUCTURE_TYPE_PAST_PRESENTATION_TIMING_EXT;
            timings[i].pNext = NULL;
            timings[i].presentStageCount = 4;
            timings[i].pPresentStages = stage_storage[i];
        }
        VkPastPresentationTimingPropertiesEXT props = {
            .sType = VK_STRUCTURE_TYPE_PAST_PRESENTATION_TIMING_PROPERTIES_EXT,
            .pNext = NULL,
            .presentationTimingCount = PAPO_PT_DRAIN_BATCH,
            .pPresentationTimings = timings,
        };
        VkResult res = get_past_pt(device, &query, &props);
        if (res != VK_SUCCESS && res != VK_INCOMPLETE) {
            break;
        }
        if (props.presentationTimingCount == 0) {
            break;
        }
        uint32_t rec_count = props.presentationTimingCount;
        if (rec_count > PAPO_PT_DRAIN_BATCH) {
            rec_count = PAPO_PT_DRAIN_BATCH;
        }

        pthread_mutex_lock(&g_lock);
        SwapchainData *sc = find_swapchain_locked(swapchain);
        for (uint32_t i = 0; i < rec_count; i++) {
            const VkPastPresentationTimingEXT *t = &timings[i];
            DrainEventRecord *rec = &records[i];
            memset(rec, 0, sizeof(*rec));
            rec->present_id = t->presentId;
            rec->time_domain = t->timeDomain;
            rec->report_complete = t->reportComplete ? 1u : 0u;
            for (uint32_t s = 0; s < t->presentStageCount; s++) {
                rec->stages_mask |= (uint32_t)t->pPresentStages[s].stage;
                if (t->pPresentStages[s].stage == VK_PRESENT_STAGE_QUEUE_OPERATIONS_END_BIT_EXT) {
                    rec->queue_end_ns = t->pPresentStages[s].time;
                } else if (t->pPresentStages[s].stage ==
                           VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT) {
                    rec->first_pixel_out_ns = t->pPresentStages[s].time;
                } else if (t->pPresentStages[s].stage ==
                           VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_VISIBLE_BIT_EXT) {
                    rec->first_pixel_visible_ns = t->pPresentStages[s].time;
                }
            }
            bool presented = (rec->first_pixel_out_ns != 0) || (rec->first_pixel_visible_ns != 0);
            rec->presented = presented ? 1u : 0u;
            if (sc) {
                InflightPresent *slot = &sc->ring[t->presentId % PAPO_PT_RING_CAP];
                if (slot->occupied && slot->present_id == t->presentId) {
                    rec->matched = *slot;
                    if (t->reportComplete == VK_TRUE) {
                        slot->occupied = false;
                    }
                }
                if (t->reportComplete == VK_TRUE) {
                    sc->total_completed++;
                    if (presented) {
                        sc->total_presented++;
                    } else {
                        sc->total_discarded++;
                    }
                }
            }
        }
        pthread_mutex_unlock(&g_lock);

        pthread_mutex_lock(&g_io_lock);
        for (uint32_t i = 0; i < rec_count; i++) {
            const DrainEventRecord *r = &records[i];
            char line[512];
            int n = snprintf(
                line,
                sizeof(line),
                "EVENT,%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%u,%u,%" PRIu64 ",%" PRIu64 ",%" PRIu64
                ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",%" PRIu64 ",0x%x,%s,%u,%u\n",
                r->present_id,
                r->matched.latch_seq,
                r->matched.submit_mono_ns,
                r->matched.tile_count,
                r->matched.valid_mask,
                r->matched.tile_pts[0],
                r->matched.tile_pts[1],
                r->matched.tile_pts[2],
                r->matched.tile_pts[3],
                r->first_pixel_out_ns,
                r->first_pixel_visible_ns,
                r->queue_end_ns,
                r->stages_mask,
                time_domain_name(r->time_domain),
                r->report_complete,
                r->presented
            );
            if (n > 0) {
                size_t len = (size_t)n;
                if (len >= sizeof(line)) {
                    len = sizeof(line) - 1;
                }
                out_write_locked(line, len);
            }
        }
        pthread_mutex_unlock(&g_io_lock);

        if (res != VK_INCOMPLETE) {
            break;
        }
    }
}

static void emit_swapchain_summary_and_retire(
    VkDevice device,
    VkSwapchainKHR swapchain,
    PFN_vkGetPastPresentationTimingEXT get_past_pt
) {
    drain_swapchain_timings(device, swapchain, get_past_pt);

    SwapchainData snap = {0};
    bool had_sc = false;
    pthread_mutex_lock(&g_lock);
    SwapchainData *sc = find_swapchain_locked(swapchain);
    if (sc) {
        snap = *sc;
        sc->in_use = false;
        had_sc = true;
    }
    pthread_mutex_unlock(&g_lock);

    if (had_sc) {
        io_printf(
            "SUMMARY,total_presents=%" PRIu64 ",total_completed=%" PRIu64
            ",total_presented=%" PRIu64 ",total_discarded=%" PRIu64
            ",overflow=%u,supported_stages=0x%x,domain=%s\n",
            snap.total_presents,
            snap.total_completed,
            snap.total_presented,
            snap.total_discarded,
            snap.overflow,
            snap.supported_stages,
            time_domain_name(snap.chosen_domain)
        );
        io_flush();
    }
}

static VKAPI_ATTR VkResult VKAPI_CALL papo_CreateInstance(
    const VkInstanceCreateInfo *pCreateInfo,
    const VkAllocationCallbacks *pAllocator,
    VkInstance *pInstance
) {
    if (!pCreateInfo || !pInstance) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    VkLayerInstanceCreateInfo *chain = (VkLayerInstanceCreateInfo *)pCreateInfo->pNext;
    while (chain != NULL &&
           !(chain->sType == VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO &&
             chain->function == VK_LAYER_LINK_INFO)) {
        chain = (VkLayerInstanceCreateInfo *)chain->pNext;
    }
    if (chain == NULL || chain->u.pLayerInfo == NULL) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    PFN_vkGetInstanceProcAddr next_gipa = chain->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    chain->u.pLayerInfo = chain->u.pLayerInfo->pNext;
    PFN_vkCreateInstance next_create =
        (PFN_vkCreateInstance)next_gipa(VK_NULL_HANDLE, "vkCreateInstance");
    if (!next_create) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }

    uint32_t ext_count = pCreateInfo->enabledExtensionCount;
    const char **exts = (const char **)calloc((size_t)ext_count + 4u, sizeof(const char *));
    if (!exts) {
        return VK_ERROR_OUT_OF_HOST_MEMORY;
    }
    for (uint32_t i = 0; i < ext_count; i++) {
        exts[i] = pCreateInfo->ppEnabledExtensionNames[i];
    }
    if (!has_ext(exts, ext_count, VK_KHR_GET_SURFACE_CAPABILITIES_2_EXTENSION_NAME)) {
        exts[ext_count++] = VK_KHR_GET_SURFACE_CAPABILITIES_2_EXTENSION_NAME;
    }
    if (!has_ext(exts, ext_count, VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME)) {
        exts[ext_count++] = VK_KHR_GET_PHYSICAL_DEVICE_PROPERTIES_2_EXTENSION_NAME;
    }

    VkInstanceCreateInfo mod = *pCreateInfo;
    mod.enabledExtensionCount = ext_count;
    mod.ppEnabledExtensionNames = exts;

    VkResult res = next_create(&mod, pAllocator, pInstance);
    free(exts);
    if (res != VK_SUCCESS) {
        return res;
    }

    PFN_vkDestroyInstance next_destroy =
        (PFN_vkDestroyInstance)next_gipa(*pInstance, "vkDestroyInstance");
    PFN_vkEnumeratePhysicalDevices next_enum_phys =
        (PFN_vkEnumeratePhysicalDevices)next_gipa(*pInstance, "vkEnumeratePhysicalDevices");
    PFN_vkGetPhysicalDeviceFeatures2 next_feats2 =
        (PFN_vkGetPhysicalDeviceFeatures2)next_gipa(*pInstance, "vkGetPhysicalDeviceFeatures2");
    if (!next_feats2) {
        next_feats2 = (PFN_vkGetPhysicalDeviceFeatures2)next_gipa(
            *pInstance, "vkGetPhysicalDeviceFeatures2KHR"
        );
    }
    PFN_vkGetPhysicalDeviceSurfaceCapabilities2KHR next_surf_caps2 =
        (PFN_vkGetPhysicalDeviceSurfaceCapabilities2KHR)next_gipa(
            *pInstance, "vkGetPhysicalDeviceSurfaceCapabilities2KHR"
        );
    PFN_vkEnumerateDeviceExtensionProperties next_enum_dev =
        (PFN_vkEnumerateDeviceExtensionProperties)next_gipa(
            *pInstance, "vkEnumerateDeviceExtensionProperties"
        );

    init_io();

    pthread_mutex_lock(&g_lock);
    for (size_t i = 0; i < PAPO_PT_MAX_INSTANCES; i++) {
        if (!g_instances[i].in_use) {
            InstanceData *inst = &g_instances[i];
            memset(inst, 0, sizeof(*inst));
            inst->in_use = true;
            inst->instance = *pInstance;
            inst->next_gipa = next_gipa;
            inst->next_destroy_instance = next_destroy;
            inst->next_enum_phys_devs = next_enum_phys;
            inst->next_get_phys_features2 = next_feats2;
            inst->next_get_surf_caps2 = next_surf_caps2;
            inst->next_enum_dev_exts = next_enum_dev;
            break;
        }
    }
    pthread_mutex_unlock(&g_lock);

    io_printf(
        "INSTANCE,get_surf_caps2=%u,get_phys_features2=%u\n",
        next_surf_caps2 ? 1u : 0u,
        next_feats2 ? 1u : 0u
    );
    return VK_SUCCESS;
}

static VKAPI_ATTR void VKAPI_CALL papo_DestroyInstance(
    VkInstance instance,
    const VkAllocationCallbacks *pAllocator
) {
    PFN_vkDestroyInstance next_destroy = NULL;
    bool any_left = false;
    pthread_mutex_lock(&g_lock);
    InstanceData *inst = find_instance_locked(instance);
    if (inst) {
        next_destroy = inst->next_destroy_instance;
        for (size_t i = 0; i < PAPO_PT_MAX_PHYS_DEVS; i++) {
            if (g_phys_devs[i].in_use && g_phys_devs[i].inst == inst) {
                g_phys_devs[i].in_use = false;
            }
        }
        inst->in_use = false;
    }
    for (size_t i = 0; i < PAPO_PT_MAX_INSTANCES; i++) {
        any_left |= g_instances[i].in_use;
    }
    pthread_mutex_unlock(&g_lock);

    if (!any_left) {
        close_io();
    } else {
        io_flush();
    }
    if (next_destroy) {
        next_destroy(instance, pAllocator);
    }
}

static VKAPI_ATTR VkResult VKAPI_CALL papo_EnumeratePhysicalDevices(
    VkInstance instance,
    uint32_t *pPhysicalDeviceCount,
    VkPhysicalDevice *pPhysicalDevices
) {
    PFN_vkEnumeratePhysicalDevices next_fn = NULL;
    pthread_mutex_lock(&g_lock);
    InstanceData *inst = find_instance_locked(instance);
    if (inst) {
        next_fn = inst->next_enum_phys_devs;
    }
    pthread_mutex_unlock(&g_lock);
    if (!next_fn) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    VkResult res = next_fn(instance, pPhysicalDeviceCount, pPhysicalDevices);
    if ((res == VK_SUCCESS || res == VK_INCOMPLETE) && pPhysicalDevices && pPhysicalDeviceCount) {
        pthread_mutex_lock(&g_lock);
        InstanceData *owner = find_instance_locked(instance);
        if (owner) {
            for (uint32_t d = 0; d < *pPhysicalDeviceCount; d++) {
                bool found = false;
                for (size_t i = 0; i < PAPO_PT_MAX_PHYS_DEVS; i++) {
                    if (g_phys_devs[i].in_use && g_phys_devs[i].phys == pPhysicalDevices[d]) {
                        g_phys_devs[i].inst = owner;
                        found = true;
                        break;
                    }
                }
                if (!found) {
                    for (size_t i = 0; i < PAPO_PT_MAX_PHYS_DEVS; i++) {
                        if (!g_phys_devs[i].in_use) {
                            g_phys_devs[i].in_use = true;
                            g_phys_devs[i].phys = pPhysicalDevices[d];
                            g_phys_devs[i].inst = owner;
                            break;
                        }
                    }
                }
            }
        }
        pthread_mutex_unlock(&g_lock);
    }
    return res;
}

static VKAPI_ATTR VkResult VKAPI_CALL papo_CreateDevice(
    VkPhysicalDevice physicalDevice,
    const VkDeviceCreateInfo *pCreateInfo,
    const VkAllocationCallbacks *pAllocator,
    VkDevice *pDevice
) {
    if (!pCreateInfo || !pDevice) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    VkLayerDeviceCreateInfo *chain = (VkLayerDeviceCreateInfo *)pCreateInfo->pNext;
    while (chain != NULL &&
           !(chain->sType == VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO &&
             chain->function == VK_LAYER_LINK_INFO)) {
        chain = (VkLayerDeviceCreateInfo *)chain->pNext;
    }
    if (chain == NULL || chain->u.pLayerInfo == NULL) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    PFN_vkGetInstanceProcAddr next_gipa = chain->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    PFN_vkGetDeviceProcAddr next_gdpa = chain->u.pLayerInfo->pfnNextGetDeviceProcAddr;
    chain->u.pLayerInfo = chain->u.pLayerInfo->pNext;

    pthread_mutex_lock(&g_lock);
    InstanceData *inst = find_instance_by_phys_locked(physicalDevice);
    VkInstance vk_inst = inst ? inst->instance : VK_NULL_HANDLE;
    PFN_vkGetPhysicalDeviceFeatures2 get_feats2 = inst ? inst->next_get_phys_features2 : NULL;
    PFN_vkEnumerateDeviceExtensionProperties enum_exts = inst ? inst->next_enum_dev_exts : NULL;
    pthread_mutex_unlock(&g_lock);

    PFN_vkCreateDevice next_create = (PFN_vkCreateDevice)next_gipa(vk_inst, "vkCreateDevice");
    if (!next_create) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }

    uint32_t ext_count = pCreateInfo->enabledExtensionCount;
    const char **exts = (const char **)calloc((size_t)ext_count + 4u, sizeof(const char *));
    if (!exts) {
        return VK_ERROR_OUT_OF_HOST_MEMORY;
    }
    for (uint32_t i = 0; i < ext_count; i++) {
        exts[i] = pCreateInfo->ppEnabledExtensionNames[i];
    }

    bool wants_swapchain = has_ext(exts, ext_count, VK_KHR_SWAPCHAIN_EXTENSION_NAME);
    bool add_present_id2 = false;
    bool add_present_timing = false;
    bool add_calibrated = false;

    if (wants_swapchain && enum_exts && get_feats2) {
        uint32_t avail_count = 0;
        if (enum_exts(physicalDevice, NULL, &avail_count, NULL) == VK_SUCCESS && avail_count > 0) {
            VkExtensionProperties *avail =
                (VkExtensionProperties *)calloc(avail_count, sizeof(VkExtensionProperties));
            if (avail && enum_exts(physicalDevice, NULL, &avail_count, avail) == VK_SUCCESS) {
                bool has_id2 = dev_supports_ext(avail, avail_count, VK_KHR_PRESENT_ID_2_EXTENSION_NAME);
                bool has_pt = dev_supports_ext(avail, avail_count, VK_EXT_PRESENT_TIMING_EXTENSION_NAME);
                bool has_cal =
                    dev_supports_ext(avail, avail_count, VK_KHR_CALIBRATED_TIMESTAMPS_EXTENSION_NAME);
                if (has_id2 && has_pt && has_cal) {
                    VkPhysicalDevicePresentTimingFeaturesEXT pt_q = {
                        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_TIMING_FEATURES_EXT,
                    };
                    VkPhysicalDevicePresentId2FeaturesKHR id2_q = {
                        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR,
                        .pNext = &pt_q,
                    };
                    VkPhysicalDeviceFeatures2 f2 = {
                        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2,
                        .pNext = &id2_q,
                    };
                    get_feats2(physicalDevice, &f2);
                    if (id2_q.presentId2 && pt_q.presentTiming) {
                        add_present_id2 = true;
                        add_present_timing = true;
                        add_calibrated = true;
                    }
                }
            }
            free(avail);
        }
    }

    if (add_present_id2 && !has_ext(exts, ext_count, VK_KHR_PRESENT_ID_2_EXTENSION_NAME)) {
        exts[ext_count++] = VK_KHR_PRESENT_ID_2_EXTENSION_NAME;
    }
    if (add_calibrated && !has_ext(exts, ext_count, VK_KHR_CALIBRATED_TIMESTAMPS_EXTENSION_NAME)) {
        exts[ext_count++] = VK_KHR_CALIBRATED_TIMESTAMPS_EXTENSION_NAME;
    }
    if (add_present_timing && !has_ext(exts, ext_count, VK_EXT_PRESENT_TIMING_EXTENSION_NAME)) {
        exts[ext_count++] = VK_EXT_PRESENT_TIMING_EXTENSION_NAME;
    }

    VkDeviceCreateInfo mod = *pCreateInfo;
    mod.enabledExtensionCount = ext_count;
    mod.ppEnabledExtensionNames = exts;

    VkPhysicalDevicePresentTimingFeaturesEXT pt_feat = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_TIMING_FEATURES_EXT,
        .pNext = (void *)mod.pNext,
        .presentTiming = VK_TRUE,
    };
    VkPhysicalDevicePresentId2FeaturesKHR id2_feat = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR,
        .pNext = (void *)mod.pNext,
        .presentId2 = VK_TRUE,
    };

    const VkPhysicalDevicePresentTimingFeaturesEXT *existing_pt =
        (const VkPhysicalDevicePresentTimingFeaturesEXT *)pnext_find_stype(
            mod.pNext, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_TIMING_FEATURES_EXT
        );
    bool pt_feature_active = false;
    if (add_present_timing) {
        if (existing_pt) {
            pt_feature_active = (existing_pt->presentTiming == VK_TRUE);
        } else {
            pt_feat.pNext = (void *)mod.pNext;
            mod.pNext = &pt_feat;
            pt_feature_active = true;
        }
    }

    const VkPhysicalDevicePresentId2FeaturesKHR *existing_id2 =
        (const VkPhysicalDevicePresentId2FeaturesKHR *)pnext_find_stype(
            mod.pNext, VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR
        );
    bool id2_feature_active = false;
    if (add_present_id2) {
        if (existing_id2) {
            id2_feature_active = (existing_id2->presentId2 == VK_TRUE);
        } else {
            id2_feat.pNext = (void *)mod.pNext;
            mod.pNext = &id2_feat;
            id2_feature_active = true;
        }
    }

    VkResult res = next_create(physicalDevice, &mod, pAllocator, pDevice);
    free(exts);
    if (res != VK_SUCCESS) {
        return res;
    }

    PFN_vkDestroyDevice d_destroy = (PFN_vkDestroyDevice)next_gdpa(*pDevice, "vkDestroyDevice");
    PFN_vkCreateSwapchainKHR d_create_sc =
        (PFN_vkCreateSwapchainKHR)next_gdpa(*pDevice, "vkCreateSwapchainKHR");
    PFN_vkDestroySwapchainKHR d_destroy_sc =
        (PFN_vkDestroySwapchainKHR)next_gdpa(*pDevice, "vkDestroySwapchainKHR");
    PFN_vkQueuePresentKHR d_present =
        (PFN_vkQueuePresentKHR)next_gdpa(*pDevice, "vkQueuePresentKHR");
    PFN_vkSetSwapchainPresentTimingQueueSizeEXT d_set_pt_q =
        (PFN_vkSetSwapchainPresentTimingQueueSizeEXT)next_gdpa(
            *pDevice, "vkSetSwapchainPresentTimingQueueSizeEXT"
        );
    PFN_vkGetSwapchainTimingPropertiesEXT d_get_pt_props =
        (PFN_vkGetSwapchainTimingPropertiesEXT)next_gdpa(
            *pDevice, "vkGetSwapchainTimingPropertiesEXT"
        );
    PFN_vkGetSwapchainTimeDomainPropertiesEXT d_get_pt_dom =
        (PFN_vkGetSwapchainTimeDomainPropertiesEXT)next_gdpa(
            *pDevice, "vkGetSwapchainTimeDomainPropertiesEXT"
        );
    PFN_vkGetPastPresentationTimingEXT d_get_past_pt =
        (PFN_vkGetPastPresentationTimingEXT)next_gdpa(*pDevice, "vkGetPastPresentationTimingEXT");
    PFN_vkGetCalibratedTimestampsKHR d_get_cal =
        (PFN_vkGetCalibratedTimestampsKHR)next_gdpa(*pDevice, "vkGetCalibratedTimestampsKHR");
    if (!d_get_cal && vk_inst != VK_NULL_HANDLE) {
        d_get_cal =
            (PFN_vkGetCalibratedTimestampsKHR)next_gipa(vk_inst, "vkGetCalibratedTimestampsKHR");
    }
    bool pt_ready = pt_feature_active && d_set_pt_q && d_get_past_pt;

    pthread_mutex_lock(&g_lock);
    inst = find_instance_by_phys_locked(physicalDevice);
    for (size_t i = 0; i < PAPO_PT_MAX_DEVICES; i++) {
        if (!g_devices[i].in_use) {
            DeviceData *dev = &g_devices[i];
            memset(dev, 0, sizeof(*dev));
            dev->in_use = true;
            dev->device = *pDevice;
            dev->phys = physicalDevice;
            dev->inst = inst;
            dev->next_gdpa = next_gdpa;
            dev->next_destroy_device = d_destroy;
            dev->next_create_swapchain = d_create_sc;
            dev->next_destroy_swapchain = d_destroy_sc;
            dev->next_queue_present = d_present;
            dev->next_set_pt_queue_size = d_set_pt_q;
            dev->next_get_pt_props = d_get_pt_props;
            dev->next_get_pt_domains = d_get_pt_dom;
            dev->next_get_past_pt = d_get_past_pt;
            dev->next_get_calibrated_timestamps = d_get_cal;
            dev->present_id2_ext_enabled = id2_feature_active;
            dev->present_timing_ext_enabled = pt_ready;
            break;
        }
    }
    pthread_mutex_unlock(&g_lock);

    io_printf(
        "DEVICE,wants_swapchain=%u,id2=%u,pt=%u,has_pt_fns=%u\n",
        wants_swapchain ? 1u : 0u,
        id2_feature_active ? 1u : 0u,
        pt_feature_active ? 1u : 0u,
        pt_ready ? 1u : 0u
    );
    io_flush();
    return VK_SUCCESS;
}

static VKAPI_ATTR void VKAPI_CALL papo_DestroyDevice(
    VkDevice device,
    const VkAllocationCallbacks *pAllocator
) {
    PFN_vkDestroyDevice next_destroy = NULL;
    PFN_vkGetPastPresentationTimingEXT get_past_pt = NULL;
    VkSwapchainKHR owned_scs[PAPO_PT_MAX_SWAPCHAINS];
    size_t sc_count = 0;

    pthread_mutex_lock(&g_lock);
    DeviceData *dev = find_device_locked(device);
    if (dev) {
        next_destroy = dev->next_destroy_device;
        get_past_pt = dev->next_get_past_pt;
        for (size_t i = 0; i < PAPO_PT_MAX_SWAPCHAINS; i++) {
            if (g_swapchains[i].in_use && g_swapchains[i].device == device) {
                owned_scs[sc_count++] = g_swapchains[i].swapchain;
            }
        }
    }
    pthread_mutex_unlock(&g_lock);

    for (size_t i = 0; i < sc_count; i++) {
        emit_swapchain_summary_and_retire(device, owned_scs[i], get_past_pt);
    }

    pthread_mutex_lock(&g_lock);
    dev = find_device_locked(device);
    if (dev) {
        dev->in_use = false;
    }
    pthread_mutex_unlock(&g_lock);

    io_flush();
    if (next_destroy) {
        next_destroy(device, pAllocator);
    }
}

static VKAPI_ATTR VkResult VKAPI_CALL papo_CreateSwapchainKHR(
    VkDevice device,
    const VkSwapchainCreateInfoKHR *pCreateInfo,
    const VkAllocationCallbacks *pAllocator,
    VkSwapchainKHR *pSwapchain
) {
    if (!pCreateInfo || !pSwapchain) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    pthread_mutex_lock(&g_lock);
    DeviceData *dev = find_device_locked(device);
    PFN_vkCreateSwapchainKHR next_create = dev ? dev->next_create_swapchain : NULL;
    PFN_vkSetSwapchainPresentTimingQueueSizeEXT set_pt_q =
        dev ? dev->next_set_pt_queue_size : NULL;
    PFN_vkGetSwapchainTimingPropertiesEXT get_pt_props = dev ? dev->next_get_pt_props : NULL;
    PFN_vkGetSwapchainTimeDomainPropertiesEXT get_pt_dom = dev ? dev->next_get_pt_domains : NULL;
    PFN_vkGetCalibratedTimestampsKHR get_cal = dev ? dev->next_get_calibrated_timestamps : NULL;
    bool pt_ready = dev && dev->present_timing_ext_enabled && dev->present_id2_ext_enabled;
    InstanceData *inst = dev ? dev->inst : NULL;
    VkPhysicalDevice phys = dev ? dev->phys : VK_NULL_HANDLE;
    PFN_vkGetPhysicalDeviceSurfaceCapabilities2KHR get_surf_caps2 =
        inst ? inst->next_get_surf_caps2 : NULL;
    pthread_mutex_unlock(&g_lock);

    if (!next_create) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }

    bool enable_timing = false;
    VkPresentStageFlagsEXT supported_stages = 0;
    if (pt_ready && get_surf_caps2) {
        VkPresentTimingSurfaceCapabilitiesEXT pt_caps = {
            .sType = VK_STRUCTURE_TYPE_PRESENT_TIMING_SURFACE_CAPABILITIES_EXT,
        };
        VkSurfaceCapabilitiesPresentId2KHR id2_caps = {
            .sType = VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_ID_2_KHR,
            .pNext = &pt_caps,
        };
        VkSurfaceCapabilities2KHR sc2 = {
            .sType = VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_2_KHR,
            .pNext = &id2_caps,
        };
        VkPhysicalDeviceSurfaceInfo2KHR info2 = {
            .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SURFACE_INFO_2_KHR,
            .surface = pCreateInfo->surface,
        };
        if (get_surf_caps2(phys, &info2, &sc2) == VK_SUCCESS) {
            supported_stages = pt_caps.presentStageQueries;
            if (id2_caps.presentId2Supported && pt_caps.presentTimingSupported &&
                (supported_stages & (VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT |
                                     VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_VISIBLE_BIT_EXT)) != 0) {
                enable_timing = true;
            }
        }
    } else if (pt_ready) {
        enable_timing = true;
        supported_stages = VK_PRESENT_STAGE_QUEUE_OPERATIONS_END_BIT_EXT |
                           VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT;
    }

    VkSwapchainCreateInfoKHR mod = *pCreateInfo;
    if (enable_timing) {
        mod.flags |= VK_SWAPCHAIN_CREATE_PRESENT_ID_2_BIT_KHR |
                     VK_SWAPCHAIN_CREATE_PRESENT_TIMING_BIT_EXT;
    }

    VkResult res = next_create(device, &mod, pAllocator, pSwapchain);
    if (res != VK_SUCCESS && enable_timing && !get_surf_caps2) {
        enable_timing = false;
        res = next_create(device, pCreateInfo, pAllocator, pSwapchain);
    }
    if (res != VK_SUCCESS) {
        return res;
    }

    VkPresentStageFlagsEXT requested_stages = supported_stages &
        (VK_PRESENT_STAGE_QUEUE_OPERATIONS_END_BIT_EXT |
         VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT |
         VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_VISIBLE_BIT_EXT);
    if (requested_stages == 0) {
        requested_stages = VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT;
    }

    VkTimeDomainKHR domains[16];
    uint64_t domain_ids[16];
    uint32_t domain_count = 0;
    VkTimeDomainKHR chosen_domain = VK_TIME_DOMAIN_CLOCK_MONOTONIC_KHR;
    uint64_t chosen_domain_id = 0;
    uint64_t refresh_duration_ns = 0;
    uint64_t refresh_interval_ns = 0;
    int64_t cal_delta_ns = 0;
    uint64_t cal_max_dev_ns = 0;
    uint32_t cal_ok = 0;

    if (enable_timing) {
        if (!set_pt_q || set_pt_q(device, *pSwapchain, 64) != VK_SUCCESS) {
            enable_timing = false;
        }
        if (enable_timing && get_pt_props) {
            VkSwapchainTimingPropertiesEXT tprops = {
                .sType = VK_STRUCTURE_TYPE_SWAPCHAIN_TIMING_PROPERTIES_EXT,
            };
            uint64_t tcounter = 0;
            if (get_pt_props(device, *pSwapchain, &tprops, &tcounter) == VK_SUCCESS) {
                refresh_duration_ns = tprops.refreshDuration;
                refresh_interval_ns = tprops.refreshInterval;
            }
        }
        if (enable_timing && get_pt_dom) {
            VkSwapchainTimeDomainPropertiesEXT dprops = {
                .sType = VK_STRUCTURE_TYPE_SWAPCHAIN_TIME_DOMAIN_PROPERTIES_EXT,
                .timeDomainCount = 16,
                .pTimeDomains = domains,
                .pTimeDomainIds = domain_ids,
            };
            uint64_t dcounter = 0;
            if (get_pt_dom(device, *pSwapchain, &dprops, &dcounter) == VK_SUCCESS &&
                dprops.timeDomainCount > 0) {
                domain_count = dprops.timeDomainCount;
                if (domain_count > 16) {
                    domain_count = 16;
                }
                chosen_domain = domains[0];
                chosen_domain_id = domain_ids[0];
                for (uint32_t d = 0; d < domain_count; d++) {
                    if (domains[d] == VK_TIME_DOMAIN_CLOCK_MONOTONIC_KHR) {
                        chosen_domain = domains[d];
                        chosen_domain_id = domain_ids[d];
                        break;
                    }
                }
            }
        }
        if (enable_timing && get_cal) {
            VkPresentStageFlagBitsEXT cal_stage =
                (supported_stages & VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT)
                    ? VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_OUT_BIT_EXT
                    : VK_PRESENT_STAGE_IMAGE_FIRST_PIXEL_VISIBLE_BIT_EXT;
            bool is_local_domain =
                (chosen_domain == VK_TIME_DOMAIN_SWAPCHAIN_LOCAL_EXT) ||
                (chosen_domain == VK_TIME_DOMAIN_PRESENT_STAGE_LOCAL_EXT);
            VkSwapchainCalibratedTimestampInfoEXT sc_cal = {
                .sType = VK_STRUCTURE_TYPE_SWAPCHAIN_CALIBRATED_TIMESTAMP_INFO_EXT,
                .swapchain = *pSwapchain,
                .presentStage = cal_stage,
                .timeDomainId = chosen_domain_id,
            };
            VkCalibratedTimestampInfoKHR cal_infos[2] = {
                {
                    .sType = VK_STRUCTURE_TYPE_CALIBRATED_TIMESTAMP_INFO_KHR,
                    .timeDomain = VK_TIME_DOMAIN_CLOCK_MONOTONIC_KHR,
                },
                {
                    .sType = VK_STRUCTURE_TYPE_CALIBRATED_TIMESTAMP_INFO_KHR,
                    .pNext = is_local_domain ? &sc_cal : NULL,
                    .timeDomain = chosen_domain,
                },
            };
            uint64_t timestamps[2] = {0, 0};
            if (get_cal(device, 2, cal_infos, timestamps, &cal_max_dev_ns) == VK_SUCCESS) {
                cal_ok = 1;
                cal_delta_ns = (int64_t)(timestamps[1] - timestamps[0]);
            }
        }
    }

    pthread_mutex_lock(&g_lock);
    for (size_t i = 0; i < PAPO_PT_MAX_SWAPCHAINS; i++) {
        if (!g_swapchains[i].in_use) {
            SwapchainData *sc = &g_swapchains[i];
            memset(sc, 0, sizeof(*sc));
            sc->in_use = true;
            sc->swapchain = *pSwapchain;
            sc->device = device;
            sc->timing_enabled = enable_timing;
            sc->supported_stages = supported_stages;
            sc->requested_stages = requested_stages;
            sc->chosen_domain = chosen_domain;
            sc->chosen_domain_id = chosen_domain_id;
            sc->refresh_duration_ns = refresh_duration_ns;
            sc->refresh_interval_ns = refresh_interval_ns;
            break;
        }
    }
    pthread_mutex_unlock(&g_lock);

    for (uint32_t d = 0; d < domain_count; d++) {
        io_printf(
            "SWAPCHAIN_DOMAIN,idx=%u,domain=%s,domain_id=%" PRIu64 "\n",
            d,
            time_domain_name(domains[d]),
            domain_ids[d]
        );
    }
    io_printf(
        "SWAPCHAIN,timing_enabled=%u,supported_stages=0x%x,requested_stages=0x%x,"
        "domain=%s,domain_id=%" PRIu64 ",refresh_duration_ns=%" PRIu64 ",refresh_interval_ns=%" PRIu64
        ",cal_ok=%u,cal_delta_ns=%" PRId64 ",cal_max_dev_ns=%" PRIu64 "\n",
        enable_timing ? 1u : 0u,
        supported_stages,
        requested_stages,
        time_domain_name(chosen_domain),
        chosen_domain_id,
        refresh_duration_ns,
        refresh_interval_ns,
        cal_ok,
        cal_delta_ns,
        cal_max_dev_ns
    );
    io_flush();
    return VK_SUCCESS;
}

static VKAPI_ATTR void VKAPI_CALL papo_DestroySwapchainKHR(
    VkDevice device,
    VkSwapchainKHR swapchain,
    const VkAllocationCallbacks *pAllocator
) {
    PFN_vkDestroySwapchainKHR next_destroy = NULL;
    PFN_vkGetPastPresentationTimingEXT get_past_pt = NULL;
    pthread_mutex_lock(&g_lock);
    DeviceData *dev = find_device_locked(device);
    if (dev) {
        next_destroy = dev->next_destroy_swapchain;
        get_past_pt = dev->next_get_past_pt;
    }
    pthread_mutex_unlock(&g_lock);

    emit_swapchain_summary_and_retire(device, swapchain, get_past_pt);
    if (next_destroy) {
        next_destroy(device, swapchain, pAllocator);
    }
}

static VKAPI_ATTR VkResult VKAPI_CALL papo_QueuePresentKHR(
    VkQueue queue,
    const VkPresentInfoKHR *pPresentInfo
) {
    if (!pPresentInfo) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }

    VkSwapchainKHR target_sc =
        (pPresentInfo->swapchainCount > 0 && pPresentInfo->pSwapchains)
            ? pPresentInfo->pSwapchains[0]
            : VK_NULL_HANDLE;

    pthread_mutex_lock(&g_lock);
    DeviceData *dev = NULL;
    SwapchainData *sc = (target_sc != VK_NULL_HANDLE) ? find_swapchain_locked(target_sc) : NULL;
    if (sc) {
        dev = find_device_locked(sc->device);
    }
    if (!dev) {
        for (size_t i = 0; i < PAPO_PT_MAX_DEVICES; i++) {
            if (g_devices[i].in_use && g_devices[i].next_queue_present) {
                dev = &g_devices[i];
                break;
            }
        }
    }
    PFN_vkQueuePresentKHR next_present = dev ? dev->next_queue_present : NULL;
    PFN_vkGetPastPresentationTimingEXT get_past_pt = dev ? dev->next_get_past_pt : NULL;
    VkDevice vk_dev = dev ? dev->device : VK_NULL_HANDLE;
    bool timing_enabled = (dev && sc && sc->timing_enabled && pPresentInfo->swapchainCount == 1);
    pthread_mutex_unlock(&g_lock);

    if (!next_present) {
        return VK_ERROR_DEVICE_LOST;
    }

    if (timing_enabled && get_past_pt) {
        drain_swapchain_timings(vk_dev, target_sc, get_past_pt);
    }

    VkPresentInfoKHR mod = *pPresentInfo;
    uint64_t present_id = 0;
    VkPresentId2KHR id2_info;
    VkPresentTimingInfoEXT timing_info;
    VkPresentTimingsInfoEXT timings_info;
    bool chained = false;

    bool already_chained =
        pnext_find_stype(mod.pNext, VK_STRUCTURE_TYPE_PRESENT_ID_2_KHR) != NULL ||
        pnext_find_stype(mod.pNext, VK_STRUCTURE_TYPE_PRESENT_TIMINGS_INFO_EXT) != NULL;

    if (timing_enabled && !already_chained) {
        PapoPtLatch latch;
        read_latch(&latch);
        uint64_t submit_mono_ns = monotonic_ns();

        pthread_mutex_lock(&g_lock);
        sc = find_swapchain_locked(target_sc);
        if (sc && sc->timing_enabled) {
            present_id = ++sc->next_present_id;
            sc->total_presents++;
            InflightPresent *slot = &sc->ring[present_id % PAPO_PT_RING_CAP];
            if (slot->occupied) {
                sc->overflow = 1;
            }
            slot->occupied = true;
            slot->present_id = present_id;
            slot->latch_seq = latch.seq;
            slot->submit_mono_ns = submit_mono_ns;
            slot->tile_count = latch.tile_count;
            slot->valid_mask = latch.valid_mask;
            for (uint32_t t = 0; t < 4; t++) {
                slot->tile_pts[t] = (t < latch.tile_count) ? latch.tiles[t].pts_ns : 0;
            }
            id2_info = (VkPresentId2KHR){
                .sType = VK_STRUCTURE_TYPE_PRESENT_ID_2_KHR,
                .pNext = mod.pNext,
                .swapchainCount = 1,
                .pPresentIds = &present_id,
            };
            timing_info = (VkPresentTimingInfoEXT){
                .sType = VK_STRUCTURE_TYPE_PRESENT_TIMING_INFO_EXT,
                .pNext = NULL,
                .flags = 0,
                .targetTime = 0,
                .timeDomainId = sc->chosen_domain_id,
                .presentStageQueries = sc->requested_stages,
                .targetTimeDomainPresentStage = 0,
            };
            timings_info = (VkPresentTimingsInfoEXT){
                .sType = VK_STRUCTURE_TYPE_PRESENT_TIMINGS_INFO_EXT,
                .pNext = &id2_info,
                .swapchainCount = 1,
                .pTimingInfos = &timing_info,
            };
            mod.pNext = &timings_info;
            chained = true;
        }
        pthread_mutex_unlock(&g_lock);
    }

    VkResult res = next_present(queue, &mod);
    if (chained && res != VK_SUCCESS && res != VK_SUBOPTIMAL_KHR) {
        pthread_mutex_lock(&g_lock);
        sc = find_swapchain_locked(target_sc);
        if (sc) {
            InflightPresent *slot = &sc->ring[present_id % PAPO_PT_RING_CAP];
            if (slot->occupied && slot->present_id == present_id) {
                slot->occupied = false;
            }
            if (sc->total_presents > 0) {
                sc->total_presents--;
            }
        }
        pthread_mutex_unlock(&g_lock);
    }
    return res;
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL papo_pt_GetDeviceProcAddr(VkDevice device, const char *pName) {
    if (!pName) {
        return NULL;
    }
    if (strcmp(pName, "vkGetDeviceProcAddr") == 0) {
        return (PFN_vkVoidFunction)papo_pt_GetDeviceProcAddr;
    }
    if (strcmp(pName, "vkDestroyDevice") == 0) {
        return (PFN_vkVoidFunction)papo_DestroyDevice;
    }
    if (strcmp(pName, "vkCreateSwapchainKHR") == 0) {
        return (PFN_vkVoidFunction)papo_CreateSwapchainKHR;
    }
    if (strcmp(pName, "vkDestroySwapchainKHR") == 0) {
        return (PFN_vkVoidFunction)papo_DestroySwapchainKHR;
    }
    if (strcmp(pName, "vkQueuePresentKHR") == 0) {
        return (PFN_vkVoidFunction)papo_QueuePresentKHR;
    }

    PFN_vkGetDeviceProcAddr next_gdpa = NULL;
    pthread_mutex_lock(&g_lock);
    DeviceData *dev = find_device_locked(device);
    if (dev) {
        next_gdpa = dev->next_gdpa;
    }
    pthread_mutex_unlock(&g_lock);
    return next_gdpa ? next_gdpa(device, pName) : NULL;
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL papo_pt_GetInstanceProcAddr(VkInstance instance, const char *pName) {
    if (!pName) {
        return NULL;
    }
    if (strcmp(pName, "vkGetInstanceProcAddr") == 0) {
        return (PFN_vkVoidFunction)papo_pt_GetInstanceProcAddr;
    }
    if (strcmp(pName, "vkCreateInstance") == 0) {
        return (PFN_vkVoidFunction)papo_CreateInstance;
    }
    if (strcmp(pName, "vkDestroyInstance") == 0) {
        return (PFN_vkVoidFunction)papo_DestroyInstance;
    }
    if (strcmp(pName, "vkEnumeratePhysicalDevices") == 0) {
        return (PFN_vkVoidFunction)papo_EnumeratePhysicalDevices;
    }
    if (strcmp(pName, "vkCreateDevice") == 0) {
        return (PFN_vkVoidFunction)papo_CreateDevice;
    }
    if (strcmp(pName, "vkGetDeviceProcAddr") == 0) {
        return (PFN_vkVoidFunction)papo_pt_GetDeviceProcAddr;
    }
    if (strcmp(pName, "vkDestroyDevice") == 0) {
        return (PFN_vkVoidFunction)papo_DestroyDevice;
    }
    if (strcmp(pName, "vkCreateSwapchainKHR") == 0) {
        return (PFN_vkVoidFunction)papo_CreateSwapchainKHR;
    }
    if (strcmp(pName, "vkDestroySwapchainKHR") == 0) {
        return (PFN_vkVoidFunction)papo_DestroySwapchainKHR;
    }
    if (strcmp(pName, "vkQueuePresentKHR") == 0) {
        return (PFN_vkVoidFunction)papo_QueuePresentKHR;
    }

    PFN_vkGetInstanceProcAddr next_gipa = NULL;
    pthread_mutex_lock(&g_lock);
    InstanceData *inst = find_instance_locked(instance);
    if (inst) {
        next_gipa = inst->next_gipa;
    }
    pthread_mutex_unlock(&g_lock);
    return next_gipa ? next_gipa(instance, pName) : NULL;
}

VKAPI_ATTR VkResult VKAPI_CALL vkNegotiateLoaderLayerInterfaceVersion(
    VkNegotiateLayerInterface *pVersionStruct
) {
    if (!pVersionStruct || pVersionStruct->sType != LAYER_NEGOTIATE_INTERFACE_STRUCT) {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    if (pVersionStruct->loaderLayerInterfaceVersion > CURRENT_LOADER_LAYER_INTERFACE_VERSION) {
        pVersionStruct->loaderLayerInterfaceVersion = CURRENT_LOADER_LAYER_INTERFACE_VERSION;
    }
    pVersionStruct->pfnGetInstanceProcAddr = papo_pt_GetInstanceProcAddr;
    pVersionStruct->pfnGetDeviceProcAddr = papo_pt_GetDeviceProcAddr;
    pVersionStruct->pfnGetPhysicalDeviceProcAddr = NULL;
    return VK_SUCCESS;
}
