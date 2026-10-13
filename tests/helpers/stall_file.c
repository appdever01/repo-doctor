#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef int (*open_fn)(const char *, int, ...);

static int inspect_open(const char *symbol, const char *path, int flags, mode_t mode) {
    open_fn original = (open_fn)dlsym(RTLD_NEXT, symbol);
    const char *target = getenv("REPO_DOCTOR_TEST_STALL_FILE");
    const char *marker = getenv("REPO_DOCTOR_TEST_STALL_MARKER");
    if (target && marker && strcmp(path, target) == 0) {
        int file = original(marker, O_WRONLY | O_CREAT | O_TRUNC, 0600);
        if (file >= 0) {
            write(file, "started", 7);
            close(file);
        }
        sleep(60);
    }
    return original(path, flags, mode);
}

int open(const char *path, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list args;
        va_start(args, flags);
        mode = va_arg(args, mode_t);
        va_end(args);
    }
    return inspect_open("open", path, flags, mode);
}

int open64(const char *path, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list args;
        va_start(args, flags);
        mode = va_arg(args, mode_t);
        va_end(args);
    }
    return inspect_open("open64", path, flags, mode);
}
