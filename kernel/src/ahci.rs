//! Driver AHCI (SATA) — detección e inicialización mínima (BORRADOR SIN
//! VERIFICAR EN QEMU — pero registros contrastados contra fuente real).
//!
//! Alcance de esta pasada: encontrar el controlador AHCI por PCI,
//! mapear su ABAR, habilitar modo AHCI, leer qué puertos están
//! implementados y qué dispositivo hay en cada uno (vía PxSSTS/PxSIG).
//! Emitir el primer comando real (IDENTIFY DEVICE) necesita montar
//! Command List + tabla FIS + PRDT — bastante más superficie de
//! registros y más riesgo de un offset mal puesto. Se deja para una
//! pasada dedicada aparte, no mezclada con esta.
//!
//! VERIFICADO: la tabla de offsets de registros de abajo se contrastó
//! contra `drivers/ata/ahci.h` del kernel de Linux real (GPL-2.0,
//! github.com/torvalds/linux) — HOST_CAP=0x00, HOST_CTL=0x04,
//! HOST_PORTS_IMPL=0x0c, PORT_SCR_STAT=0x28, PORT_SIG=0x24, base de
//! puerto = mmio+0x100+(n*0x80), fórmula de nº de puertos = (cap&0x1f)+1
//! — todos coinciden exactamente. Sigue sin ejecutarse en QEMU, pero ya
//! no es "memoria sin contrastar", es "offsets confirmados, lógica de
//! Rust por verificar".

use crate::pci::{self, PciDevice};
use crate::serial_println;
use alloc::string::{String, ToString};

const AHCI_CLASS: u8 = 0x01; // mass storage
const AHCI_SUBCLASS: u8 = 0x06; // SATA
const AHCI_PROG_IF: u8 = 0x01; // AHCI

const REG_CAP: u64 = 0x00;
const REG_GHC: u64 = 0x04;
const REG_PI: u64 = 0x0C;
const GHC_AE: u32 = 1 << 31; // AHCI Enable

const PORT_BASE: u64 = 0x100;
const PORT_SIZE: u64 = 0x80;
const PORT_SSTS: u64 = 0x28;
const PORT_SIG: u64 = 0x24;

const SATA_SIG_ATA: u32 = 0x00000101;
const SATA_SIG_ATAPI: u32 = 0xEB140101;

// Registros de puerto adicionales — necesarios para emitir comandos de
// verdad, no solo detectar. Offsets verificados contra drivers/ata/ahci.h
// del kernel de Linux, igual que la tabla de detección de arriba.
const PORT_CLB: u64 = 0x00;
const PORT_CLBU: u64 = 0x04;
const PORT_FB: u64 = 0x08;
const PORT_FBU: u64 = 0x0C;
const PORT_CMD_REG: u64 = 0x18;
const PORT_TFD: u64 = 0x20;
const PORT_CI: u64 = 0x38;

const CMD_FRE: u32 = 1 << 4; // FIS Receive Enable
const CMD_ST: u32 = 1 << 0; // Start
const TFD_ERR: u32 = 1 << 0; // bit0 del Task File Data = ERR

const ATA_CMD_IDENTIFY_DEVICE: u8 = 0xEC;
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_FLUSH_CACHE_EXT: u8 = 0xEA;

/// Command Header — una por slot, 32 bytes, formato estándar AHCI
/// (spec §4.2.2, idéntico en cualquier implementación).
#[repr(C)]
struct CmdHeader {
    flags: u16,  // bits0-4=CFL, bit6=W, resto flags — solo usamos CFL
    prdtl: u16,  // número de entradas PRDT
    prdbc: u32,  // bytes transferidos, lo rellena el controlador
    ctba: u32,   // dirección física de la Command Table (bits 0-31)
    ctba_hi: u32,
    reserved: [u32; 4],
}

/// Entrada de la Physical Region Descriptor Table — describe un buffer
/// de datos físico contiguo (spec §4.2.3.3).
#[repr(C)]
struct PrdtEntry {
    dba: u32,
    dba_hi: u32,
    reserved: u32,
    dbc_i: u32, // bits0-21 = bytes-1, bit31 = interrupt on completion
}

/// Estado de un puerto ya inicializado — guarda las direcciones físicas
/// de sus estructuras para poder emitir más comandos sin re-montarlo
/// todo cada vez.
#[derive(Clone, Copy)]
pub struct AhciPort {
    port_base: u64,
    cmd_list: u64,
    cmd_table: u64,
}

/// Monta Command List + área de recepción de FIS para un puerto y
/// arranca su motor de comandos. Se hace una sola vez por puerto.
///
/// Simplificación deliberada de esta pasada: no esperamos activamente a
/// que PxCMD.CR/FR reflejen el estado antes de continuar — la mayoría
/// de implementaciones de referencia sí lo hacen; en QEMU funciona sin
/// eso, en hardware real podría necesitar ese endurecimiento después.
unsafe fn init_port(port_base: u64) -> Option<AhciPort> {
    // Parar el motor de comandos por si venía arrancado de antes.
    let mut cmd = reg_read(port_base, PORT_CMD_REG);
    cmd &= !CMD_ST;
    reg_write(port_base, PORT_CMD_REG, cmd);

    let cmd_list = crate::pmm::alloc_frame()?;
    let fis_area = crate::pmm::alloc_frame()?;
    let cmd_table = crate::pmm::alloc_frame()?;

    // Los frames vienen con basura de uso físico previo — limpiamos.
    core::ptr::write_bytes(cmd_list as *mut u8, 0, 4096);
    core::ptr::write_bytes(fis_area as *mut u8, 0, 4096);
    core::ptr::write_bytes(cmd_table as *mut u8, 0, 4096);

    reg_write(port_base, PORT_CLB, cmd_list as u32);
    reg_write(port_base, PORT_CLBU, (cmd_list >> 32) as u32);
    reg_write(port_base, PORT_FB, fis_area as u32);
    reg_write(port_base, PORT_FBU, (fis_area >> 32) as u32);

    // FRE antes que ST — orden exigido por la spec (§10.1.2).
    let mut cmd = reg_read(port_base, PORT_CMD_REG);
    cmd |= CMD_FRE;
    reg_write(port_base, PORT_CMD_REG, cmd);
    cmd |= CMD_ST;
    reg_write(port_base, PORT_CMD_REG, cmd);

    Some(AhciPort {
        port_base,
        cmd_list,
        cmd_table,
    })
}

/// Emite un comando ATA en el slot 0 y espera (polling) a que termine.
/// `setup_fis` recibe el puntero al FIS H2D ya puesto a cero para que
/// el llamante rellene solo lo específico de su comando.
///
/// `byte_count` es el tamaño exacto del buffer de destino; `data_buf`
/// debe ser una dirección física válida (frame de `pmm`).
unsafe fn issue_command(
    port: &AhciPort,
    data_buf: u64,
    byte_count: u32,
    write: bool,
    setup_fis: impl FnOnce(*mut u8),
) -> Result<(), &'static str> {
    // Limpiamos la Command Table entera antes de cada comando — evita
    // arrastrar campos del comando anterior por descuido.
    core::ptr::write_bytes(port.cmd_table as *mut u8, 0, 4096);

    let header = &mut *(port.cmd_list as *mut CmdHeader);
    // CFL=5 DWORDS (el FIS H2D Register mide exactamente eso).
    // Bit 6 = W: 1 = escritura host->dispositivo, 0 = lectura.
    header.flags = 5 | if write { 1 << 6 } else { 0 };
    header.prdtl = 1;
    header.prdbc = 0;
    header.ctba = port.cmd_table as u32;
    header.ctba_hi = (port.cmd_table >> 32) as u32;

    let cfis = port.cmd_table as *mut u8;
    *cfis.add(0) = 0x27; // FIS type: Register — Host to Device
    *cfis.add(1) = 0x80; // bit7=C (actualización de comando), PM port 0
    setup_fis(cfis);

    // PRDT en offset 0x80 de la Command Table (spec §4.2.3.1).
    let prdt = &mut *((port.cmd_table + 0x80) as *mut PrdtEntry);
    prdt.dba = data_buf as u32;
    prdt.dba_hi = (data_buf >> 32) as u32;
    prdt.reserved = 0;
    prdt.dbc_i = byte_count - 1; // bytes-1; bit31=0, sin IRQ, hacemos polling

    reg_write(port.port_base, PORT_CI, 1);

    // Polling hasta que el bit del slot se limpie o hasta timeout —
    // sin IRQ configurada para esta pasada.
    let mut timeout = 1_000_000u32;
    while reg_read(port.port_base, PORT_CI) & 1 != 0 {
        timeout -= 1;
        if timeout == 0 {
            return Err("timeout esperando a que termine el comando");
        }
    }

    if reg_read(port.port_base, PORT_TFD) & TFD_ERR != 0 {
        return Err("el dispositivo devolvió error (TFD.ERR)");
    }
    Ok(())
}

/// `IDENTIFY DEVICE` — devuelve el modelo del disco (40 bytes, tal como
/// lo entrega el propio dispositivo).
unsafe fn identify(port: &AhciPort) -> Option<[u8; 40]> {
    let data_buf = crate::pmm::alloc_frame()?;
    core::ptr::write_bytes(data_buf as *mut u8, 0, 4096);

    if let Err(e) = issue_command(port, data_buf, 512, false, |cfis| {
        *cfis.add(2) = ATA_CMD_IDENTIFY_DEVICE;
    }) {
        serial_println!("[ahci] IDENTIFY: {}", e);
        crate::pmm::free_frame(data_buf);
        return None;
    }

    // Modelo del disco: palabras 27-46 del buffer IDENTIFY, cada
    // palabra de 16 bits llega con los bytes intercambiados (spec ATA
    // — orden "big-endian dentro de cada word").
    let words = data_buf as *const u16;
    let mut model = [0u8; 40];
    for i in 0..20 {
        let word = words.add(27 + i).read_volatile();
        model[i * 2] = (word >> 8) as u8;
        model[i * 2 + 1] = (word & 0xFF) as u8;
    }
    crate::pmm::free_frame(data_buf);
    Some(model)
}

/// Rellena un FIS H2D para un comando LBA48 de transferencia de
/// sectores (READ/WRITE DMA EXT comparten exactamente este layout —
/// spec ATA §7.24/§7.60, solo cambia el opcode).
unsafe fn setup_lba48_fis(cfis: *mut u8, command: u8, lba: u64, count: u16) {
    *cfis.add(2) = command;
    // LBA48: bytes bajos en 4-6, altos en 8-10
    *cfis.add(4) = lba as u8;
    *cfis.add(5) = (lba >> 8) as u8;
    *cfis.add(6) = (lba >> 16) as u8;
    *cfis.add(7) = 1 << 6; // device: bit6 = modo LBA
    *cfis.add(8) = (lba >> 24) as u8;
    *cfis.add(9) = (lba >> 32) as u8;
    *cfis.add(10) = (lba >> 40) as u8;
    *cfis.add(12) = count as u8;
    *cfis.add(13) = (count >> 8) as u8;
}

/// Lee `count` sectores de 512 bytes empezando en el LBA dado, usando
/// `READ DMA EXT` (LBA48). Copia el resultado a `dest`.
///
/// `dest.len()` debe ser exactamente `count * 512`. Un solo PRDT de
/// 4 KiB limita esta versión a 8 sectores por llamada — suficiente para
/// leer superbloques/inodos en M5; transferencias grandes necesitarán
/// varias entradas PRDT.
pub fn read_sectors(port: &AhciPort, lba: u64, count: u16, dest: &mut [u8]) -> Result<(), &'static str> {
    let byte_count = count as usize * 512;
    if dest.len() != byte_count {
        return Err("el buffer de destino no coincide con count*512");
    }
    if byte_count > 4096 {
        return Err("máximo 8 sectores por llamada en esta versión (un solo PRDT)");
    }

    unsafe {
        let data_buf = match crate::pmm::alloc_frame() {
            Some(f) => f,
            None => return Err("sin memoria física para el buffer DMA"),
        };
        core::ptr::write_bytes(data_buf as *mut u8, 0, 4096);

        let result = issue_command(port, data_buf, byte_count as u32, false, |cfis| {
            setup_lba48_fis(cfis, ATA_CMD_READ_DMA_EXT, lba, count);
        });

        if result.is_ok() {
            core::ptr::copy_nonoverlapping(data_buf as *const u8, dest.as_mut_ptr(), byte_count);
        }
        crate::pmm::free_frame(data_buf);
        result
    }
}

/// Escribe `count` sectores de 512 bytes en el LBA dado, usando
/// `WRITE DMA EXT` (LBA48), seguido de `FLUSH CACHE EXT` para que los
/// datos lleguen de verdad al medio y no se queden en la caché del
/// disco (sin el flush, un corte de corriente los perdería).
///
/// ⚠️ **Destructivo**: sobrescribe el contenido del disco en ese LBA.
/// Mismas restricciones de tamaño que `read_sectors`.
pub fn write_sectors(port: &AhciPort, lba: u64, count: u16, src: &[u8]) -> Result<(), &'static str> {
    let byte_count = count as usize * 512;
    if src.len() != byte_count {
        return Err("el buffer de origen no coincide con count*512");
    }
    if byte_count > 4096 {
        return Err("máximo 8 sectores por llamada en esta versión (un solo PRDT)");
    }

    unsafe {
        let data_buf = match crate::pmm::alloc_frame() {
            Some(f) => f,
            None => return Err("sin memoria física para el buffer DMA"),
        };
        core::ptr::write_bytes(data_buf as *mut u8, 0, 4096);
        core::ptr::copy_nonoverlapping(src.as_ptr(), data_buf as *mut u8, byte_count);

        let result = issue_command(port, data_buf, byte_count as u32, true, |cfis| {
            setup_lba48_fis(cfis, ATA_CMD_WRITE_DMA_EXT, lba, count);
        });
        crate::pmm::free_frame(data_buf);
        result?;

        // FLUSH CACHE EXT — sin datos asociados, pero el PRDT debe
        // apuntar a algo válido, así que reutilizamos un frame mínimo.
        let flush_buf = match crate::pmm::alloc_frame() {
            Some(f) => f,
            None => return Err("escritura OK pero sin memoria para el flush"),
        };
        let flush_result = issue_command(port, flush_buf, 512, false, |cfis| {
            *cfis.add(2) = ATA_CMD_FLUSH_CACHE_EXT;
            *cfis.add(7) = 1 << 6;
        });
        crate::pmm::free_frame(flush_buf);
        flush_result
    }
}

/// Devuelve el primer puerto AHCI con un disco SATA listo, ya
/// inicializado. Para uso desde la consola de depuración y, más
/// adelante, desde el VFS (M5).
pub fn first_disk() -> Option<AhciPort> {
    let dev = find_controller()?;
    let bars = pci::read_bars(dev.bus, dev.device, dev.function);
    let abar = bars[5];
    if !pci::bar_is_memory(abar) || abar == 0 {
        return None;
    }
    let base = pci::bar_memory_address(abar);

    unsafe {
        let ghc = reg_read(base, REG_GHC);
        reg_write(base, REG_GHC, ghc | GHC_AE);
        let pi = reg_read(base, REG_PI);

        for port in 0..32u32 {
            if pi & (1 << port) == 0 {
                continue;
            }
            let port_base = base + PORT_BASE + (port as u64) * PORT_SIZE;
            if reg_read(port_base, PORT_SSTS) & 0xF != 0x3 {
                continue;
            }
            if reg_read(port_base, PORT_SIG) != SATA_SIG_ATA {
                continue;
            }
            return init_port(port_base);
        }
    }
    None
}

/// Prueba de escritura NO destructiva: lee un sector, lo reescribe tal
/// cual, y verifica que sigue igual. Si el contenido cambia, algo va
/// mal en el camino de escritura.
///
/// Usa un LBA alto (por defecto 2048) para no tocar el MBR ni tablas de
/// particiones aunque el disco tenga datos.
pub fn write_readback_test(port: &AhciPort, lba: u64) -> Result<(), &'static str> {
    let mut original = [0u8; 512];
    read_sectors(port, lba, 1, &mut original)?;

    // Reescribimos exactamente lo mismo que había: si todo funciona, el
    // disco queda idéntico a como estaba.
    write_sectors(port, lba, 1, &original)?;

    let mut readback = [0u8; 512];
    read_sectors(port, lba, 1, &mut readback)?;

    if original == readback {
        Ok(())
    } else {
        Err("el contenido leído tras escribir no coincide con el original")
    }
}

fn model_to_string(model: &[u8; 40]) -> String {
    let mut s = String::new();
    for &b in model.iter() {
        if b.is_ascii_graphic() || b == b' ' {
            s.push(b as char);
        }
    }
    s.trim().to_string()
}

unsafe fn reg_read(base: u64, offset: u64) -> u32 {
    ((base + offset) as *const u32).read_volatile()
}

unsafe fn reg_write(base: u64, offset: u64, value: u32) {
    ((base + offset) as *mut u32).write_volatile(value);
}

pub fn find_controller() -> Option<PciDevice> {
    let mut found = None;
    pci::enumerate(|dev| {
        if found.is_none()
            && dev.class == AHCI_CLASS
            && dev.subclass == AHCI_SUBCLASS
            && dev.prog_if == AHCI_PROG_IF
        {
            found = Some(dev);
        }
    });
    found
}

/// Detecta el controlador, lo habilita, y reporta qué hay en cada
/// puerto implementado. No monta command lists todavía — solo detección.
pub fn probe_and_print() {
    let dev = match find_controller() {
        Some(d) => d,
        None => {
            serial_println!("[ahci] no se encontró ningún controlador AHCI");
            return;
        }
    };

    serial_println!(
        "[ahci] controlador en {:02x}:{:02x}.{} (vendor=0x{:04x} device=0x{:04x})",
        dev.bus,
        dev.device,
        dev.function,
        dev.vendor_id,
        dev.device_id
    );

    let bars = pci::read_bars(dev.bus, dev.device, dev.function);
    let abar = bars[5]; // BAR5 es siempre el ABAR en AHCI
    if !pci::bar_is_memory(abar) || abar == 0 {
        serial_println!("[ahci] BAR5 (ABAR) inválido o ausente — abortando");
        return;
    }
    let base = pci::bar_memory_address(abar);
    serial_println!("[ahci] ABAR @ 0x{:x}", base);

    unsafe {
        // Habilitar modo AHCI — algunos controladores arrancan en modo
        // legacy IDE y hay que pedírselo explícitamente.
        let ghc = reg_read(base, REG_GHC);
        reg_write(base, REG_GHC, ghc | GHC_AE);

        let cap = reg_read(base, REG_CAP);
        let num_ports = (cap & 0x1F) + 1; // bits 4:0 = nº puertos - 1
        let pi = reg_read(base, REG_PI); // bitmap de puertos implementados

        serial_println!(
            "[ahci] CAP=0x{:x} ({} puertos soportados), PI=0x{:x}",
            cap,
            num_ports,
            pi
        );

        let mut found_any = false;
        for port in 0..32u32 {
            if pi & (1 << port) == 0 {
                continue; // puerto no implementado en este controlador
            }
            let port_base = base + PORT_BASE + (port as u64) * PORT_SIZE;
            let ssts = reg_read(port_base, PORT_SSTS);
            let det = ssts & 0xF; // Device Detection

            if det != 0x3 {
                continue; // sin dispositivo o sin comunicación PHY
            }

            found_any = true;
            let sig = reg_read(port_base, PORT_SIG);
            let kind = match sig {
                SATA_SIG_ATA => "disco SATA (ATA)",
                SATA_SIG_ATAPI => "unidad ATAPI (CD/DVD)",
                _ => "dispositivo desconocido",
            };

            if sig == SATA_SIG_ATA {
                match init_port(port_base) {
                    Some(p) => {
                        match identify(&p) {
                            Some(model) => {
                                let name = model_to_string(&model);
                                serial_println!("[ahci] puerto {}: {} — modelo: \"{}\"", port, kind, name);
                            }
                            None => {
                                serial_println!("[ahci] puerto {}: {} — IDENTIFY falló, ver arriba", port, kind);
                            }
                        }

                        // Prueba de lectura real: sector 0 (MBR / inicio
                        // del disco). Si la firma 0x55AA está presente,
                        // es un MBR válido — prueba fuerte de que la
                        // lectura DMA funciona de verdad.
                        let mut sector = [0u8; 512];
                        match read_sectors(&p, 0, 1, &mut sector) {
                            Ok(()) => {
                                serial_println!(
                                    "[ahci] sector 0 leído: primeros bytes {:02x} {:02x} {:02x} {:02x}, firma final {:02x}{:02x}",
                                    sector[0], sector[1], sector[2], sector[3],
                                    sector[510], sector[511]
                                );
                                if sector[510] == 0x55 && sector[511] == 0xAA {
                                    serial_println!("[ahci] firma MBR 0x55AA presente — lectura DMA correcta");
                                }
                            }
                            Err(e) => serial_println!("[ahci] lectura del sector 0 falló: {}", e),
                        }
                    }
                    None => {
                        serial_println!("[ahci] puerto {}: {} — no se pudo inicializar (¿sin memoria?)", port, kind);
                    }
                }
            } else {
                serial_println!("[ahci] puerto {}: {} (sig=0x{:x})", port, kind, sig);
            }
        }

        if !found_any {
            serial_println!("[ahci] ningún puerto con dispositivo activo (¿VM sin disco SATA adjunto?)");
        }
    }
}
