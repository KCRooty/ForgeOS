// sysdeps/forgeos/generic.cpp — sysdeps de ForgeOS para mlibc v7
//
// Syscalls que divergen de Linux x86_64:
//   SYS_WRITE = 63  (Linux usa 1)
// El resto coincide con Linux x86_64.

#include <mlibc/all-sysdeps.hpp>
#include <mlibc/tcb.hpp>
#include <string.h>
#include <errno.h>
#include <time.h>

// mmap constants (x86_64, same as Linux — defined inline to avoid sys/mman.h POSIX dep)
#ifndef PROT_READ
#define PROT_READ    0x1
#define PROT_WRITE   0x2
#define MAP_PRIVATE  0x02
#define MAP_ANONYMOUS 0x20
#define MAP_FAILED   ((void *)(long)-1)
#endif

// ── Syscall helper ───────────────────────────────────────────────────────────

static inline long forge_syscall(long nr,
    long a0 = 0, long a1 = 0, long a2 = 0,
    long a3 = 0, long a4 = 0, long a5 = 0)
{
    long ret;
    register long r10 asm("r10") = a3;
    register long r8  asm("r8")  = a4;
    register long r9  asm("r9")  = a5;
    asm volatile("syscall"
        : "=a"(ret)
        : "0"(nr), "D"(a0), "S"(a1), "d"(a2), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory");
    return ret;
}

// ── Implementaciones ────────────────────────────────────────────────────────

namespace mlibc {

void Sysdeps<LibcPanic>::operator()() {
    const char *msg = "!!! mlibc panic (ForgeOS) !!!\n";
    forge_syscall(63, 2, reinterpret_cast<long>(msg), static_cast<long>(strlen(msg)));
    forge_syscall(60, -1);
    __builtin_unreachable();
}

void Sysdeps<LibcLog>::operator()(const char *msg) {
    forge_syscall(63, 2, reinterpret_cast<long>(msg), static_cast<long>(strlen(msg)));
}

void Sysdeps<Exit>::operator()(int status) {
    forge_syscall(60, status);
    __builtin_unreachable();
}

// SYS_WRITE en ForgeOS = 63 (Linux usa 1)
int Sysdeps<Write>::operator()(int fd, void const *buf, size_t size, ssize_t *ret) {
    long r = forge_syscall(63, fd, reinterpret_cast<long>(buf), static_cast<long>(size));
    if (r < 0) return static_cast<int>(-r);
    *ret = r;
    return 0;
}

int Sysdeps<Read>::operator()(int fd, void *buf, size_t size, ssize_t *ret) {
    long r = forge_syscall(0, fd, reinterpret_cast<long>(buf), static_cast<long>(size));
    if (r < 0) return static_cast<int>(-r);
    *ret = r;
    return 0;
}

int Sysdeps<Open>::operator()(const char *path, int flags, unsigned int mode, int *fd) {
    long r = forge_syscall(2, reinterpret_cast<long>(path), flags, mode);
    if (r < 0) return static_cast<int>(-r);
    *fd = static_cast<int>(r);
    return 0;
}

int Sysdeps<Close>::operator()(int fd) {
    long r = forge_syscall(3, fd);
    return r < 0 ? static_cast<int>(-r) : 0;
}

int Sysdeps<Seek>::operator()(int fd, off_t offset, int whence, off_t *new_offset) {
    long r = forge_syscall(8, fd, offset, whence);
    if (r < 0) return static_cast<int>(-r);
    *new_offset = r;
    return 0;
}

int Sysdeps<AnonAllocate>::operator()(size_t size, void **pointer) {
    long r = forge_syscall(9, 0, static_cast<long>(size),
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (r < 0 || reinterpret_cast<void *>(r) == MAP_FAILED) {
        *pointer = nullptr;
        return ENOMEM;
    }
    *pointer = reinterpret_cast<void *>(r);
    return 0;
}

int Sysdeps<AnonFree>::operator()(void *pointer, size_t size) {
    long r = forge_syscall(11, reinterpret_cast<long>(pointer), static_cast<long>(size));
    return r < 0 ? static_cast<int>(-r) : 0;
}

int Sysdeps<VmMap>::operator()(void *hint, size_t size, int prot, int flags,
    int fd, off_t offset, void **window)
{
    long r = forge_syscall(9,
        reinterpret_cast<long>(hint), static_cast<long>(size),
        prot, flags, fd, static_cast<long>(offset));
    if (r < 0) return static_cast<int>(-r);
    *window = reinterpret_cast<void *>(r);
    return 0;
}

int Sysdeps<VmUnmap>::operator()(void *pointer, size_t size) {
    long r = forge_syscall(11, reinterpret_cast<long>(pointer), static_cast<long>(size));
    return r < 0 ? static_cast<int>(-r) : 0;
}

int Sysdeps<TcbSet>::operator()(void *pointer) {
    // ARCH_SET_FS = 0x1002, SYS_ARCH_PRCTL = 158
    long r = forge_syscall(158, 0x1002, reinterpret_cast<long>(pointer));
    return r < 0 ? static_cast<int>(-r) : 0;
}

// Futex: ForgeOS SYS_FUTEX = 202 (igual que Linux)
int Sysdeps<FutexWait>::operator()(int *pointer, int expected, timespec const *timeout) {
    forge_syscall(202,
        reinterpret_cast<long>(pointer),
        0 /* FUTEX_WAIT */,
        expected,
        reinterpret_cast<long>(timeout));
    return 0;
}

int Sysdeps<FutexWake>::operator()(int *pointer, bool all) {
    forge_syscall(202,
        reinterpret_cast<long>(pointer),
        1 /* FUTEX_WAKE */,
        all ? INT_MAX : 1);
    return 0;
}

// clock_gettime — SYS_CLOCK_GETTIME = 228 (igual que Linux)
int Sysdeps<ClockGet>::operator()(int clock_id, time_t *secs, long *nanos) {
    struct { long tv_sec; long tv_nsec; } ts{};
    long r = forge_syscall(228, clock_id, reinterpret_cast<long>(&ts));
    if (r < 0) return static_cast<int>(-r);
    *secs  = ts.tv_sec;
    *nanos = ts.tv_nsec;
    return 0;
}

int Sysdeps<Isatty>::operator()(int) {
    return 0; // siempre true (todo es tty por ahora)
}

} // namespace mlibc
