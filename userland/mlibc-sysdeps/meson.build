# meson.build para los sysdeps de ForgeOS en mlibc.
# Colocar este directorio en: <mlibc-src>/sysdeps/forgeos/
# y añadir 'forgeos' a la lista de OSes en mlibc/meson.build.
#
# Desde CachyOS (requiere meson, ninja, clang con target x86_64-elf):
#   git clone https://github.com/managarm/mlibc
#   cp -r <este-directorio>/* mlibc/sysdeps/forgeos/
#   meson setup build --cross-file cross/x86_64-forgeos.txt -Dheaders_only=false
#   ninja -C build

rtlib_src += files('forgeos/generic.cpp')
