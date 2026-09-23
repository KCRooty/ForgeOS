//! Escáner de particiones GPT/MBR + detección de filesystem (M5,
//! milestone `partinfo-ext4-preempt`).
//!
//! Igual que `ext2.rs`: cada campo se lee por offset de byte explícito
//! (`from_le_bytes`), sin `#[repr(C)]`, para no repetir el susto de
//! padding del target-spec JSON. Solo lectura — no escribe tablas de
//! particiones ni las modifica.
//!
//! Detección de filesystem por firma real en el propio contenido de la
//! partición, no por el byte de tipo de la tabla (que puede estar
//! desactualizado o ser genérico como 0x83 "Linux" sin decir si es
//! ext2/3/4): superbloque ext2 a `inicio+1024` con sus feature flags
//! para distinguir ext2/ext3/ext4, `"FAT32   "` en el boot sector de
//! FAT32, `"NTFS    "` para NTFS, y el magic `_BHRfS_M` del superbloque
//! de btrfs en su offset físico fijo (0x10040).

use crate::ahci::{self, AhciPort};
use alloc::string::String;
use alloc::vec::Vec;

const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];
const GPT_SIGNATURE: [u8; 8] = *b"EFI PART";
const MBR_TYPE_GPT_PROTECTIVE: u8 = 0xEE;

const EXT2_MAGIC: u16 = 0xEF53;
const EXT2_FEATURE_COMPAT_HAS_JOURNAL: u32 = 0x0004;
const EXT2_FEATURE_INCOMPAT_EXTENTS: u32 = 0x0040;
const EXT2_FEATURE_INCOMPAT_64BIT: u32 = 0x0080;

const BTRFS_SUPERBLOCK_OFFSET: u64 = 0x10040;
const BTRFS_MAGIC: [u8; 8] = *b"_BHRfS_M";

fn u16_at(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn u64_at(buf: &[u8], off: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[off..off + 8]);
    u64::from_le_bytes(b)
}

pub enum TableKind {
    Mbr,
    Gpt,
}

pub struct Partition {
    pub index: u32,
    pub start_lba: u64,
    pub sector_count: u64,
    /// Byte de tipo MBR (informativo, ver nota de cabecera) — `None`
    /// en GPT, donde el nombre es más significativo.
    pub mbr_type: Option<u8>,
    /// Nombre de la partición GPT (UTF-16LE decodificado) — `None` en MBR.
    pub gpt_name: Option<String>,
    pub filesystem: &'static str,
}

/// Detecta la firma real del filesystem al inicio de una partición
/// (o de un disco entero sin tabla de particiones, pasando `start_lba=0`).
pub fn detect_filesystem(port: &AhciPort, start_lba: u64) -> &'static str {
    let byte_base = start_lba * 512;

    if let Ok(sb) = ahci::read_bytes(port, byte_base + 1024, 1024) {
        if sb.len() == 1024 && u16_at(&sb, 56) == EXT2_MAGIC {
            let feature_compat = u32_at(&sb, 92);
            let feature_incompat = u32_at(&sb, 96);
            if feature_incompat & (EXT2_FEATURE_INCOMPAT_EXTENTS | EXT2_FEATURE_INCOMPAT_64BIT) != 0 {
                return "ext4";
            }
            if feature_compat & EXT2_FEATURE_COMPAT_HAS_JOURNAL != 0 {
                return "ext3";
            }
            return "ext2";
        }
    }

    if let Ok(boot) = ahci::read_bytes(port, byte_base, 512) {
        if boot.len() == 512 && boot[510..512] == MBR_SIGNATURE {
            if boot[82..90] == *b"FAT32   " {
                return "fat32";
            }
            if boot[3..11] == *b"NTFS    " {
                return "ntfs";
            }
        }
    }

    if let Ok(sb) = ahci::read_bytes(port, byte_base + BTRFS_SUPERBLOCK_OFFSET, 8) {
        if sb == BTRFS_MAGIC {
            return "btrfs";
        }
    }

    "desconocido"
}

fn decode_gpt_name(raw: &[u8]) -> String {
    let mut units = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.chunks_exact(2) {
        let u = u16::from_le_bytes([chunk[0], chunk[1]]);
        if u == 0 {
            break;
        }
        units.push(u);
    }
    core::char::decode_utf16(units.into_iter())
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect()
}

fn scan_gpt(port: &AhciPort) -> Result<Vec<Partition>, &'static str> {
    let hdr = ahci::read_bytes(port, 512, 512)?; // LBA 1
    if hdr[0..8] != GPT_SIGNATURE {
        return Err("cabecera GPT: magic \"EFI PART\" no coincide");
    }
    let part_entry_lba = u64_at(&hdr, 72);
    let num_entries = u32_at(&hdr, 80);
    let entry_size = u32_at(&hdr, 84) as usize;
    if entry_size < 128 {
        return Err("tamaño de entrada GPT inesperado (< 128 bytes)");
    }

    let array_len = num_entries as usize * entry_size;
    let array = ahci::read_bytes(port, part_entry_lba * 512, array_len)?;

    let mut partitions = Vec::new();
    for i in 0..num_entries as usize {
        let entry = &array[i * entry_size..i * entry_size + entry_size];
        let type_guid = &entry[0..16];
        if type_guid.iter().all(|&b| b == 0) {
            continue; // entrada sin usar
        }
        let first_lba = u64_at(entry, 32);
        let last_lba = u64_at(entry, 40);
        let name = decode_gpt_name(&entry[56..128]);

        let filesystem = detect_filesystem(port, first_lba);
        partitions.push(Partition {
            index: i as u32 + 1,
            start_lba: first_lba,
            sector_count: last_lba - first_lba + 1,
            mbr_type: None,
            gpt_name: Some(name),
            filesystem,
        });
    }
    Ok(partitions)
}

fn scan_mbr(port: &AhciPort, mbr: &[u8]) -> Result<Vec<Partition>, &'static str> {
    let mut partitions = Vec::new();
    for i in 0..4u32 {
        let off = 446 + (i as usize) * 16;
        let ptype = mbr[off + 4];
        if ptype == 0 {
            continue; // entrada vacía
        }
        let start_lba = u32_at(mbr, off + 8) as u64;
        let sector_count = u32_at(mbr, off + 12) as u64;
        let filesystem = detect_filesystem(port, start_lba);
        partitions.push(Partition {
            index: i + 1,
            start_lba,
            sector_count,
            mbr_type: Some(ptype),
            gpt_name: None,
            filesystem,
        });
    }
    Ok(partitions)
}

/// Escanea la tabla de particiones del disco (detecta MBR clásico vs
/// GPT vía el MBR protector) y, para cada partición encontrada,
/// identifica su filesystem por firma real.
pub fn scan(port: &AhciPort) -> Result<(TableKind, Vec<Partition>), &'static str> {
    let mbr = ahci::read_bytes(port, 0, 512)?;
    if mbr[510..512] != MBR_SIGNATURE {
        return Err("sector 0: sin firma MBR 0x55AA — ¿disco sin particionar o sin formato reconocido?");
    }

    let is_protective = (0..4).any(|i| mbr[446 + i * 16 + 4] == MBR_TYPE_GPT_PROTECTIVE);
    if is_protective {
        Ok((TableKind::Gpt, scan_gpt(port)?))
    } else {
        Ok((TableKind::Mbr, scan_mbr(port, &mbr)?))
    }
}
