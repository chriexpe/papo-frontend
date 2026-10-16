/* Native regression: repeated isolated loads and teardown, with accessibility queries.
 * Run with G_DEBUG=fatal-criticals; see docs/channel-views-testing.md. */
#include "../papo_wpe.h"
#include <stdio.h>
#include <time.h>
#include <unistd.h>
static int ready = 0;
static void event(void* u, uint32_t k, const char* a, const char* b, double n)
{
    (void)u;
    (void)b;
    (void)n;
    printf("EVENT %u %s\n", k, a ? a : "");
    fflush(stdout);
    if (k == 3)
        ready = 1;
    if (k == 5 || k == 6)
        ready = -1;
}
static void frame(void* u, const WaterWpeFrame* f)
{
    (void)u;
    for (unsigned i = 0; i < f->n_planes; i++)
        close(f->fds[i]);
    if (f->rendering_fence_fd >= 0)
        close(f->rendering_fence_fd);
    water_wpe_frame_presented(f->token);
    water_wpe_frame_release(f->token, -1);
}
static void destroy(void* u) { (void)u; }
#include <atk/atk.h>
int main(int argc, char** argv)
{
    if (argc < 2) {
        fputs("usage: app_surface_lifecycle URL [bundled]\n", stderr);
        return 2;
    }
    char* error = NULL;
    WaterWpeRuntime* r = water_wpe_runtime_new(&error);
    if (!r) {
        puts(error);
        return 1;
    }
    for (int cycle = 0; cycle < 24; cycle++) {
        ready = 0;
        WaterWpePage* p
            = water_wpe_page_new_isolated(r, event, frame, NULL, NULL, destroy, &error, argc > 2);
        if (!p) {
            puts(error);
            return 2;
        }
        water_wpe_page_resize(p, 800, 600, 1);
        water_wpe_page_load_uri(p, argv[1]);
        time_t start = time(NULL);
        while (time(NULL) - start < 18 && !ready) {
            water_wpe_runtime_iteration(r);
            usleep(1000);
        }
        if (ready != 1)
            return 3;
        water_wpe_page_free(p);
        for (int j = 0; j < 30; j++) {
            water_wpe_runtime_iteration(r);
            usleep(1000);
        }
        AtkObject* root = atk_get_root();
        int n = atk_object_get_n_accessible_children(root);
        for (int j = 0; j < n; j++) {
            AtkObject* child = atk_object_ref_accessible_child(root, j);
            if (child)
                g_object_unref(child);
        }
        printf("CYCLE %d OK\n", cycle);
    }
    water_wpe_runtime_free(r);
    puts("24 repeated isolated load/free cycles and AT-SPI queries passed");
    return 0;
}
