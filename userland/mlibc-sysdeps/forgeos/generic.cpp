// sysdeps/forgeos/generic.cpp — glue mínimo de ForgeOS para mlibc
//
// Syscall numbers que divergen de Linux x86_64:
//   SYS_WRITE = 63  (Linux usa 1)
//   SYS_READ  = 0   (igual que Linux)
//   SYS_EXIT  = 60  (igual que Linux)
// El resto (OPEN=2, CLOSE=3, FSTAT=5, MMAP=9, MUNMAP=11, BRK=12,
// GETPID=39, ARCH_PRCTL=158, SET_TID_ADDRESS=218, FUTEX=202) coinciden.
//
// Compilar junto con mlibc desde CachyOS:
//   meson setup build --cross-file cross/x86_64-forgeos.txt
//   ninja -C build

#include <mlibc/all-sysdeps.hpp>
#include <errno.h>
#include <dirent.h>
#include <fcntl.h>

// ── Syscall helper ───────────────────────────────────────────────────────────

static inline long forge_syscall(long nr,
                                  long a0 = 0, long a1 = 0, long a2 = 0,
                                  long a3 = 0, long a4 = 0, long a5 = 0) {
    long ret;
    register long r10 asm("r10") = a3;
    register long r8  asm("r8")  = a4;
    register long r9  asm("r9")  = a5;
    asm volatile(
        "syscall"
        : "=a"(ret)
        : "0"(nr), "D"(a0), "S"(a1), "d"(a2), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

// ── Interfaces requeridas por mlibc ─────────────────────────────────────────

namespace mlibc {

// TLS: mlibc llama a esto justo al arrancar para fijar el puntero al TCB.
void sys_tcb_set(void *pointer) {
    // ARCH_SET_FS = 0x1002
    forge_syscall(158, 0x1002, reinterpret_cast<long>(pointer));
}

// exit(): termina el proceso.
[[noreturn]] void sys_exit(int status) {
    forge_syscall(60, status);
    __builtin_unreachable();
}

// write(): escribe en un fd.
// NOTA: SYS_WRITE en ForgeOS = 63, no 1.
int sys_write(int fd, void const *buf, size_t count, ssize_t *bytes_written) {
    long ret = forge_syscall(63, fd,
                              reinterpret_cast<long>(buf),
                              static_cast<long>(count));
    if (ret < 0) return static_cast<int>(-ret);
    *bytes_written = ret;
    return 0;
}

// read(): lee de un fd.
int sys_read(int fd, void *buf, size_t count, ssize_t *bytes_read) {
    long ret = forge_syscall(0, fd,
                              reinterpret_cast<long>(buf),
                              static_cast<long>(count));
    if (ret < 0) return static_cast<int>(-ret);
    *bytes_read = ret;
    return 0;
}

// open(): abre un fichero.
int sys_open(const char *path, int flags, mode_t mode, int *fd) {
    long ret = forge_syscall(2,
                              reinterpret_cast<long>(path),
                              flags, mode);
    if (ret < 0) return static_cast<int>(-ret);
    *fd = static_cast<int>(ret);
    return 0;
}

// close(): cierra un fd.
int sys_close(int fd) {
    long ret = forge_syscall(3, fd);
    return ret < 0 ? static_cast<int>(-ret) : 0;
}

// fstat(): estado de un fd.
int sys_fstat(int fd, struct stat *statbuf) {
    long ret = forge_syscall(5, fd, reinterpret_cast<long>(statbuf));
    return ret < 0 ? static_cast<int>(-ret) : 0;
}

// mmap() / munmap() para el allocator de mlibc.
void *sys_vm_map(void *hint, size_t size, int prot, int flags, int fd, off_t offset, void **window) {
    long ret = forge_syscall(9,
                              reinterpret_cast<long>(hint),
                              static_cast<long>(size),
                              prot, flags, fd,
                              static_cast<long>(offset));
    if (ret < 0) return nullptr;
    *window = reinterpret_cast<void *>(ret);
    return *window;
}

int sys_vm_unmap(void *pointer, size_t size) {
    long ret = forge_syscall(11, reinterpret_cast<long>(pointer),
                                  static_cast<long>(size));
    return ret < 0 ? static_cast<int>(-ret) : 0;
}

// getpid() — requerido por algunas partes de mlibc.
int sys_getpid(pid_t *pid) {
    long ret = forge_syscall(39);
    if (ret < 0) return static_cast<int>(-ret);
    *pid = static_cast<pid_t>(ret);
    return 0;
}

// set_tid_address() — llamado en la init de mlibc.
int sys_set_tid_address(int *pointer) {
    forge_syscall(218, reinterpret_cast<long>(pointer));
    return 0;
}

// rt_sigprocmask() — stub; señales no implementadas aún.
int sys_sigprocmask(int how, const sigset_t *set, sigset_t *old) {
    (void)how; (void)set; (void)old;
    forge_syscall(14, how,
                   reinterpret_cast<long>(set),
                   reinterpret_cast<long>(old),
                   8 /* sizeof(sigset_t) */);
    return 0;
}

} // namespace mlibc
