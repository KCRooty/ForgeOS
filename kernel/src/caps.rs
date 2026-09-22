//! Capability gating estilo `pledge(2)` de OpenBSD.
//!
//! Cada proceso declara, una única vez, qué categorías de syscall necesita.
//! El kernel congela esa máscara para toda la vida del proceso: pedir algo
//! fuera de la lista declarada es un `EPERM` permanente, sin negociación.
//! No existe "ampliar permisos más tarde" — si un proceso necesita más,
//! el diseño correcto es declararlo *antes* de arrancar, no que el kernel
//! confíe en él a mitad de ejecución.
//!
//! Ver docs/PHILOSOPHY.md §1 y docs/ARCHITECTURE.md — Modelo de seguridad.

#![allow(dead_code)]

pub type CapMask = u32;

pub const CAP_STDIO: CapMask = 1 << 0; // consola / puerto serie
pub const CAP_FS_READ: CapMask = 1 << 1;
pub const CAP_FS_WRITE: CapMask = 1 << 2;
pub const CAP_EXEC: CapMask = 1 << 3; // fork/exec de otros procesos
pub const CAP_NET: CapMask = 1 << 4;
pub const CAP_MEM_MAP: CapMask = 1 << 5; // mmap/sbrk más allá del heap inicial
pub const CAP_TIME: CapMask = 1 << 6;
pub const CAP_IPC: CapMask = 1 << 7;
pub const CAP_GFX: CapMask = 1 << 8; // framebuffer / GPU

/// Máscara con todas las capacidades — solo válida para el propio kernel
/// antes de que exista tabla de procesos real (M1-M3). A partir de M4,
/// ningún proceso de userland debería nacer con esto.
pub const CAP_UNRESTRICTED: CapMask = CapMask::MAX;

#[derive(Clone, Copy)]
pub struct Capabilities(CapMask);

/// Se intentó `pledge()` con una capacidad que la máscara actual no tenía.
/// Solo se puede reducir, nunca ampliar.
#[derive(Debug)]
pub struct PledgeExpansionDenied;

/// El dispatcher pidió una capacidad que el proceso no se pledgeó.
/// En M2+, esto termina en kill del proceso — no en un error silencioso.
#[derive(Debug)]
pub struct CapViolation {
    pub requested: CapMask,
    pub granted: CapMask,
}

impl Capabilities {
    pub const fn unrestricted() -> Self {
        Capabilities(CAP_UNRESTRICTED)
    }

    pub const fn none() -> Self {
        Capabilities(0)
    }

    /// Reduce la máscara actual a `requested`. Falla si `requested` contiene
    /// algún bit que la máscara actual no tenía ya — pledge nunca amplía.
    pub fn pledge(&mut self, requested: CapMask) -> Result<(), PledgeExpansionDenied> {
        let expanding = requested & !self.0 != 0;
        if expanding && self.0 != CAP_UNRESTRICTED {
            return Err(PledgeExpansionDenied);
        }
        self.0 = requested;
        Ok(())
    }

    #[inline]
    pub fn allows(&self, cap: CapMask) -> bool {
        self.0 & cap == cap
    }

    /// Punto de aplicación real: el dispatcher de syscalls (M2+) llama esto
    /// antes de ejecutar cualquier operación.
    pub fn enforce(&self, cap: CapMask) -> Result<(), CapViolation> {
        if self.allows(cap) {
            Ok(())
        } else {
            Err(CapViolation {
                requested: cap,
                granted: self.0,
            })
        }
    }
}
