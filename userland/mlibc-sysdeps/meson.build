# meson.build para los sysdeps de ForgeOS en mlibc v7+.
# Colocar en: <mlibc-src>/sysdeps/forgeos/meson.build

sysdep_supported_options = {}

# Headers propios de ForgeOS (sysdeps.hpp) + abi-bits del demo (compatible con ForgeOS)
rtld_include_dirs += include_directories('include')
libc_include_dirs += include_directories('include')
rtld_include_dirs += include_directories('../demo/include')
libc_include_dirs += include_directories('../demo/include')

rtld_sources += files(
    'generic.cpp',
)

libc_sources += files(
    'generic.cpp',
)
