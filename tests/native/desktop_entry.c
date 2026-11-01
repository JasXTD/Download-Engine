#include <gio/gdesktopappinfo.h>
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    GDesktopAppInfo *app = g_desktop_app_info_new_from_filename(argv[1]);
    if (!app) {
        GKeyFile *key = g_key_file_new(); GError *error = NULL;
        if (!g_key_file_load_from_file(key, argv[1], G_KEY_FILE_NONE, &error)) {
            fprintf(stderr, "Key file: %s\n", error->message);
        } else {
            gchar *value = g_key_file_get_string(key, "Desktop Entry", "Exec", &error);
            fprintf(stderr, "Decoded Exec: <%s> (%s)\n", value, error ? error->message : "valid string");
            gchar **parts = NULL; int count = 0;
            if (value && g_shell_parse_argv(value, &count, &parts, &error)) {
                for (int n = 0; n < count; ++n) fprintf(stderr, "Parsed argument: <%s>\n", parts[n]);
            }
        }
        fprintf(stderr, "Desktop entry could not be parsed\n"); return 3;
    }
    const char *executable = g_app_info_get_executable(G_APP_INFO(app));
    if (strcmp(executable, "/usr/bin/env")) {
        fprintf(stderr, "Wrong launcher: <%s>\n", executable);
        return 4;
    }
    const char *uri = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=harmless";
    GList *uris = g_list_append(NULL, (gpointer)uri);
    GError *error = NULL;
    if (!g_app_info_launch_uris(G_APP_INFO(app), uris, NULL, &error)) {
        fprintf(stderr, "Launch failed: %s\n", error->message); return 5;
    }
    gchar *bytes = NULL; gsize size = 0;
    for (int n = 0; n < 100; ++n) {
        if (g_file_get_contents(argv[3], &bytes, &size, NULL)) break;
        g_usleep(10000);
    }
    if (!bytes || size != strlen(argv[2])+1+strlen(uri)+1 ||
        memcmp(bytes, argv[2], strlen(argv[2])+1) ||
        memcmp(bytes+strlen(argv[2])+1, uri, strlen(uri)+1)) {
        fprintf(stderr, "Launched arguments differ\n"); return 6;
    }
    g_free(bytes); g_list_free(uris); g_object_unref(app);
    puts("Desktop entry parsed and launched the exact path and magnet argument.");
    return 0;
}
