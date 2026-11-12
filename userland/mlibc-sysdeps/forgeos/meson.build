# meson.build para los sysdeps de ForgeOS en mlibc v7+.
# build-mlibc.sh lo copia a <mlibc>/sysdeps/forgeos/meson.build.

# ANSI C + POSIX. De POSIX el kernel sirve procesos (fork/exec/wait/
# kill) y poco más; el resto de sysdeps faltan y sus funciones devuelven
# ENOSYS en tiempo de ejecución — ver forgeos/generic.cpp.
sysdep_supported_options = {
	'posix': true,
}

rtld_sources += files(
	'generic.cpp',
)
rtld_include_dirs += include_directories('include')

libc_sources += files(
	'entry.cpp',
	'generic.cpp',
)
libc_include_dirs += include_directories('include')

# abi-bits: ForgeOS usa la ABI de Linux x86_64 (numeración de syscalls,
# struct stat, errno...). build-mlibc.sh copia los enlaces de
# sysdeps/demo/include/abi-bits (que apuntan a abis/linux) a
# sysdeps/forgeos/include/abi-bits.
if not no_headers
	install_headers(
		'include/abi-bits/auxv.h',
		'include/abi-bits/blkcnt_t.h',
		'include/abi-bits/blksize_t.h',
		'include/abi-bits/clockid_t.h',
		'include/abi-bits/dev_t.h',
		'include/abi-bits/errno.h',
		'include/abi-bits/fcntl.h',
		'include/abi-bits/gid_t.h',
		'include/abi-bits/ino_t.h',
		'include/abi-bits/limits.h',
		'include/abi-bits/mode_t.h',
		'include/abi-bits/nlink_t.h',
		'include/abi-bits/pid_t.h',
		'include/abi-bits/seek-whence.h',
		'include/abi-bits/signal.h',
		'include/abi-bits/stat.h',
		'include/abi-bits/uid_t.h',
		'include/abi-bits/vm-flags.h',
		'include/abi-bits/wait.h',
		'include/abi-bits/riscv-hwprobe.h',
		'include/abi-bits/sigevent.h',
		'include/abi-bits/sigval.h',
		'include/abi-bits/sa_family_t.h',
		'include/abi-bits/sockaddr_storage.h',
		'include/abi-bits/sig-limits.h',
		'include/abi-bits/suseconds_t.h',
		'include/abi-bits/access.h',
		'include/abi-bits/socklen_t.h',
		'include/abi-bits/socket.h',
		'include/abi-bits/poll.h',
		'include/abi-bits/resource.h',
		'include/abi-bits/in.h',
		'include/abi-bits/rlim_t.h',
		'include/abi-bits/utsname.h',
		'include/abi-bits/fd_set.h',
		'include/abi-bits/sem.h',
		'include/abi-bits/time.h',
		'include/abi-bits/ipc.h',
		'include/abi-bits/statvfs.h',
		'include/abi-bits/fsblkcnt_t.h',
		'include/abi-bits/fsfilcnt_t.h',
		'include/abi-bits/shm.h',
		'include/abi-bits/termios.h',
		'include/abi-bits/msg.h',
		'include/abi-bits/mqueue.h',
		'include/abi-bits/utmp-defines.h',
		'include/abi-bits/utmpx.h',
		'include/abi-bits/sigset_t.h',
		'include/abi-bits/aio.h',
		'include/abi-bits/sched_param.h',
		subdir: 'abi-bits',
		follow_symlinks: true
	)
endif

if not headers_only
	crt = custom_target('crt1',
		build_by_default: true,
		command: c_compiler.cmd_array() + freestanding_hdrs_args + ['-c', '-o', '@OUTPUT@', '@INPUT@'],
		input: 'crt1.S',
		output: 'crt1.o',
		install: true,
		install_dir: get_option('libdir')
	)
endif
