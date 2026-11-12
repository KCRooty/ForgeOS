// sysdeps/forgeos/generic.cpp — sysdeps de ForgeOS para mlibc v7
//
// Syscalls que divergen de Linux x86_64:
//   SYS_WRITE = 63  (Linux usa 1)
// El resto coincide con Linux x86_64 (numeración y convención: errno
// negativo en rax).
//
// Los sysdeps que el kernel todavía no puede servir devuelven ENOSYS
// aquí mismo, sin syscall — están señalados uno a uno más abajo.

#include <mlibc/all-sysdeps.hpp>
#include <mlibc/fsfd_target.hpp>
#include <mlibc/tcb.hpp>
#include <string.h>
#include <errno.h>
#include <signal.h>
#include <time.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>

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

// ── Procesos (opción POSIX, M12) ─────────────────────────────────────────────

int Sysdeps<Fork>::operator()(pid_t *child) {
    long r = forge_syscall(57);
    if (r < 0) return static_cast<int>(-r);
    *child = static_cast<pid_t>(r);
    return 0;
}

// envp se pasa pero el kernel todavía lo ignora: el programa nuevo
// arranca sin variables de entorno.
int Sysdeps<Execve>::operator()(const char *path, char *const argv[], char *const envp[]) {
    long r = forge_syscall(59,
        reinterpret_cast<long>(path),
        reinterpret_cast<long>(argv),
        reinterpret_cast<long>(envp));
    return static_cast<int>(-r); // solo vuelve si falló
}

// wait4(pid, status, options, rusage) — rusage no se rellena.
int Sysdeps<Waitpid>::operator()(pid_t pid, int *status, int flags, struct rusage *ru, pid_t *ret_pid) {
    if (ru) memset(ru, 0, sizeof(*ru));
    long r = forge_syscall(61, pid, reinterpret_cast<long>(status), flags, 0);
    if (r < 0) return static_cast<int>(-r);
    *ret_pid = static_cast<pid_t>(r);
    return 0;
}

void Sysdeps<Yield>::operator()() {
    forge_syscall(24);
}

pid_t Sysdeps<GetPid>::operator()() {
    return static_cast<pid_t>(forge_syscall(39));
}

pid_t Sysdeps<GetPpid>::operator()() {
    return static_cast<pid_t>(forge_syscall(110));
}

// Sin hilos: el TID es el PID.
pid_t Sysdeps<GetTid>::operator()() {
    return static_cast<pid_t>(forge_syscall(39));
}

int Sysdeps<Kill>::operator()(pid_t pid, int sig) {
    long r = forge_syscall(62, pid, sig);
    return r < 0 ? static_cast<int>(-r) : 0;
}

// ── Señales ──────────────────────────────────────────────────────────────────
//
// El kernel solo conoce la acción por defecto de cada señal (terminar el
// proceso). Pedir esa acción, o consultar la actual, funciona; instalar
// un manejador propio todavía no (ENOSYS). Con esto abort() ya hace lo
// que debe: restablece SIGABRT y se la envía a sí mismo.

int Sysdeps<Sigaction>::operator()(int, const struct sigaction *__restrict act, struct sigaction *__restrict old) {
    if (old) {
        memset(old, 0, sizeof(*old));
        old->sa_handler = SIG_DFL;
    }
    if (act && act->sa_handler != SIG_DFL)
        return ENOSYS;
    return 0;
}

// rt_sigprocmask: el kernel lo acepta y no hace nada (no hay señales
// que bloquear mientras no existan manejadores).
int Sysdeps<Sigprocmask>::operator()(int how, const sigset_t *__restrict set, sigset_t *__restrict retrieve) {
    if (retrieve) memset(retrieve, 0, sizeof(*retrieve));
    long r = forge_syscall(14, how, reinterpret_cast<long>(set), 0, 8);
    return r < 0 ? static_cast<int>(-r) : 0;
}

// ── Ficheros ─────────────────────────────────────────────────────────────────

// Solo fstat(fd): el kernel no tiene todavía stat() por ruta.
int Sysdeps<Stat>::operator()(fsfd_target fsfdt, int fd, const char *, int, struct stat *statbuf) {
    if (fsfdt != fsfd_target::fd)
        return ENOSYS;
    long r = forge_syscall(5, fd, reinterpret_cast<long>(statbuf));
    return r < 0 ? static_cast<int>(-r) : 0;
}

// mlibc exige estos dos para compilar su parte POSIX; el kernel no tiene
// ni dup2() ni sockets.
int Sysdeps<Dup2>::operator()(int, int, int) {
    return ENOSYS;
}

int Sysdeps<Recvfrom>::operator()(int, void *, size_t, int, struct sockaddr *, socklen_t *, ssize_t *) {
    return ENOSYS;
}

} // namespace mlibc
