#pragma once

#include <mlibc/sysdep-signatures.hpp>

namespace mlibc {

struct ForgeOSSysdepTags :
    LibcPanic,
    LibcLog,
    Exit,
    Write,
    Read,
    Open,
    Close,
    Seek,
    AnonAllocate,
    AnonFree,
    VmMap,
    VmUnmap,
    TcbSet,
    FutexWait,
    FutexWake,
    ClockGet,
    Isatty,
    // Opción POSIX (M12)
    Fork,
    Execve,
    Waitpid,
    Yield,
    GetPid,
    GetPpid,
    GetTid,
    Kill,
    Sigaction,
    Sigprocmask,
    Stat,
    Dup2,
    Recvfrom
{};

template<typename Tag>
using Sysdeps = SysdepOf<ForgeOSSysdepTags, Tag>;

struct SysdepTraits {
    static constexpr bool usesRtNetlink = false;
};

} // namespace mlibc
