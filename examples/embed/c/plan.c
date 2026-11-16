/* Plan a 4cam project from C: print the engine's version, then plan and post the project
 * and print the start of the result. Build: see docs/embedding.md (or scripts/test-capi.sh).
 *   plan examples/knob/knob.project.json */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "fourcam.h"

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc((size_t)n + 1);
    if (buf && fread(buf, 1, (size_t)n, f) == (size_t)n) buf[n] = '\0';
    else { free(buf); buf = NULL; }
    fclose(f);
    return buf;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: plan project.json\n");
        return 2;
    }
    char *version = fourcam_version();
    printf("%s\n", version);
    fourcam_free(version);

    char *project = read_file(argv[1]);
    if (!project) {
        perror(argv[1]);
        return 1;
    }
    /* NULL machine: the project's own; no mesh; default options */
    char *result = fourcam_plan_and_post(project, NULL, NULL, 0, NULL);
    free(project);
    int failed = strncmp(result, "{\"error\"", 8) == 0;
    const char *gcode = strstr(result, "\"gcode\":\"");
    printf("%zu bytes of JSON%s\n", strlen(result), failed ? " (error)" : "");
    if (failed) printf("%s\n", result);
    else if (gcode) printf("%.120s...\n", gcode);
    fourcam_free(result);
    return failed ? 1 : 0;
}
