//! ext2 real de solo lectura (M5, milestone `ext2`).
//!
//! Encima de `ahci.rs` (que ya lee/escribe sectores reales), no del
//! bump allocator ni de nada inventado — cada campo del superbloque, el
//! descriptor de grupo y el inodo se lee por offset de byte explícito
//! (`u32::from_le_bytes`/`u16::from_le_bytes` sobre el buffer crudo, sin
//! `#[repr(C)]`) para no arrastrar el mismo tipo de sorpresa de padding
//! que ya nos costó una sesión entera con el target-spec JSON.
//!
//! Alcance de esta pasada: montar (leer superbloque + tabla de
//! descriptores de grupo), resolver rutas absolutas, listar
//! directorios, leer ficheros completos — con punteros de bloque
//! directos (0-11) **y** el indirecto simple (12). **Sin doble ni
//! triple indirecto todavía** (ficheros >~12 MiB con bloques de 1 KiB):
//! error explícito en vez de truncar en silencio si hiciera falta.
//! Solo lectura — sin escritura, sin journal (ext3/ext4 real es aparte).

use crate::ahci::{self, AhciPort};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const EXT2_MAGIC: u16 = 0xEF53;
const ROOT_INODE: u32 = 2;

const EXT2_S_IFDIR: u16 = 0x4000;

fn u16_at(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Filesystem ya montado — guarda lo necesario para traducir
/// bloque-lógico -> LBA sin releer el superbloque cada vez.
pub struct Ext2Fs {
    port: AhciPort,
    block_size: u32,
    inodes_per_group: u32,
    inode_size: u32,
    /// Primer bloque de la tabla de descriptores de grupo — siempre el
    /// bloque justo después del superbloque (`first_data_block + 1`).
    bgdt_block: u32,
}

/// Vista mínima de un inodo — solo los campos que hacen falta para
/// listar/leer: modo (tipo), tamaño, y los 15 punteros de bloque.
pub struct Inode {
    pub mode: u16,
    pub size: u32,
    pub block: [u32; 15],
}

impl Inode {
    pub fn is_dir(&self) -> bool {
        self.mode & 0xF000 == EXT2_S_IFDIR
    }
}

pub struct DirEntry {
    pub inode: u32,
    pub name: String,
    pub is_dir: bool,
}

impl Ext2Fs {
    /// Monta el filesystem que empieza en el LBA 0 del disco dado (sin
    /// tabla de particiones todavía — `partinfo.rs` es milestone
    /// aparte). Lee el superbloque (siempre en el byte 1024, sin
    /// importar el tamaño de bloque real) y valida el magic.
    pub fn mount(port: AhciPort) -> Result<Ext2Fs, &'static str> {
        let mut sb = [0u8; 1024];
        ahci::read_sectors(&port, 2, 2, &mut sb)?; // byte 1024 = sector 2

        let magic = u16_at(&sb, 56);
        if magic != EXT2_MAGIC {
            return Err("magic ext2 no coincide (0xEF53) — ¿el disco no tiene ext2 en el LBA 0?");
        }

        let log_block_size = u32_at(&sb, 24);
        let block_size = 1024u32 << log_block_size;
        if block_size > 4096 {
            // read_sectors de ahci.rs solo soporta un PRDT (máx 4 KiB) —
            // bloques más grandes necesitarían leer en varias llamadas,
            // sin verificar en esta pasada.
            return Err("tamaño de bloque > 4096 no soportado en esta versión");
        }

        let first_data_block = u32_at(&sb, 20);
        let inodes_per_group = u32_at(&sb, 40);

        let rev_level = u32_at(&sb, 76);
        let inode_size = if rev_level == 0 {
            128 // EXT2_GOOD_OLD_INODE_SIZE — rev0 no tiene el campo
        } else {
            u16_at(&sb, 88) as u32
        };

        Ok(Ext2Fs {
            port,
            block_size,
            inodes_per_group,
            inode_size,
            bgdt_block: first_data_block + 1,
        })
    }

    fn sectors_per_block(&self) -> u16 {
        (self.block_size / 512) as u16
    }

    /// Lee un bloque lógico completo. Un solo bloque (máx 4096 bytes,
    /// ya garantizado por `mount`) cabe siempre en una sola llamada a
    /// `ahci::read_sectors` — coincide justo con su límite de 8
    /// sectores por PRDT, no hace falta trocear.
    fn read_block(&self, block_num: u32) -> Result<Vec<u8>, &'static str> {
        let mut buf = vec![0u8; self.block_size as usize];
        let lba = block_num as u64 * self.sectors_per_block() as u64;
        ahci::read_sectors(&self.port, lba, self.sectors_per_block(), &mut buf)?;
        Ok(buf)
    }

    /// Localiza el bloque del inodo (grupo -> descriptor de grupo ->
    /// tabla de inodos -> offset dentro del bloque) y lo parsea.
    pub fn read_inode(&self, inum: u32) -> Result<Inode, &'static str> {
        if inum == 0 {
            return Err("inodo 0 no existe (los inodos empiezan en 1)");
        }
        let group = (inum - 1) / self.inodes_per_group;
        let index_in_group = (inum - 1) % self.inodes_per_group;

        // Descriptor de grupo: 32 bytes cada uno, empezando en
        // `bgdt_block`. Puede haber varios por bloque (ej. bloque de
        // 1024 bytes / 32 bytes = 32 descriptores por bloque).
        let descs_per_block = self.block_size / 32;
        let desc_block = self.bgdt_block + group / descs_per_block;
        let desc_offset = ((group % descs_per_block) * 32) as usize;
        let desc_buf = self.read_block(desc_block)?;
        let inode_table_block = u32_at(&desc_buf, desc_offset + 8); // bg_inode_table, offset 8 del descriptor

        let inodes_per_block = self.block_size / self.inode_size;
        let inode_block = inode_table_block + index_in_group / inodes_per_block;
        let offset_in_block = ((index_in_group % inodes_per_block) * self.inode_size) as usize;

        let buf = self.read_block(inode_block)?;
        if offset_in_block + 128 > buf.len() {
            return Err("inodo fuera de los límites del bloque leído");
        }

        let mode = u16_at(&buf, offset_in_block);
        let size = u32_at(&buf, offset_in_block + 4);
        let mut block = [0u32; 15];
        for (i, b) in block.iter_mut().enumerate() {
            *b = u32_at(&buf, offset_in_block + 40 + i * 4);
        }

        Ok(Inode { mode, size, block })
    }

    /// Lista de bloques de datos lógicos del fichero, en orden —
    /// directos (0-11) seguidos de los del indirecto simple (puntero
    /// 12). Sin doble/triple indirecto: error explícito si el fichero
    /// los necesita, en vez de devolver una lista truncada en silencio.
    fn data_blocks(&self, inode: &Inode) -> Result<Vec<u32>, &'static str> {
        let needed = (inode.size as u64).div_ceil(self.block_size as u64) as usize;
        let mut blocks = Vec::with_capacity(needed);

        for &b in inode.block[0..12].iter() {
            if blocks.len() >= needed {
                return Ok(blocks);
            }
            blocks.push(b);
        }

        if blocks.len() >= needed {
            return Ok(blocks);
        }

        let indirect_ptr = inode.block[12];
        if indirect_ptr == 0 {
            return Err("faltan bloques de datos y el indirecto simple está vacío (inodo corrupto?)");
        }
        let indirect = self.read_block(indirect_ptr)?;
        let ptrs_per_block = (self.block_size / 4) as usize;
        for i in 0..ptrs_per_block {
            if blocks.len() >= needed {
                return Ok(blocks);
            }
            blocks.push(u32_at(&indirect, i * 4));
        }

        if blocks.len() < needed {
            return Err("fichero necesita doble/triple indirecto — no soportado en esta versión");
        }
        Ok(blocks)
    }

    /// Lee el contenido completo de un fichero regular ya resuelto.
    pub fn read_inode_data(&self, inode: &Inode) -> Result<Vec<u8>, &'static str> {
        let blocks = self.data_blocks(inode)?;
        let mut data = Vec::with_capacity(inode.size as usize);
        for b in blocks {
            let block_data = self.read_block(b)?;
            data.extend_from_slice(&block_data);
        }
        data.truncate(inode.size as usize);
        Ok(data)
    }

    /// Parsea las entradas de un directorio (formato `ext2_dir_entry_2`,
    /// con `file_type` — nuestra imagen de prueba lo lleva activado).
    /// Solo bloques directos: ningún directorio de esta pasada necesita
    /// más de 12 bloques (48 KiB con bloques de 4 KiB) para ser un
    /// límite razonable de v1.
    pub fn read_dir(&self, dir_inode: &Inode) -> Result<Vec<DirEntry>, &'static str> {
        if !dir_inode.is_dir() {
            return Err("no es un directorio");
        }
        let mut entries = Vec::new();
        for &b in dir_inode.block[0..12].iter() {
            if b == 0 {
                continue; // hueco disperso (sparse) o directorio más pequeño que 12 bloques
            }
            let block = self.read_block(b)?;
            let mut off = 0usize;
            while off + 8 <= block.len() {
                let inum = u32_at(&block, off);
                let rec_len = u16_at(&block, off + 4) as usize;
                if rec_len < 8 {
                    break; // entrada corrupta — mejor parar que leer basura
                }
                let name_len = block[off + 6] as usize;
                let file_type = block[off + 7];
                if inum != 0 && name_len > 0 {
                    let name_bytes = &block[off + 8..off + 8 + name_len];
                    if let Ok(name) = core::str::from_utf8(name_bytes) {
                        entries.push(DirEntry {
                            inode: inum,
                            name: String::from(name),
                            is_dir: file_type == 2, // EXT2_FT_DIR
                        });
                    }
                }
                off += rec_len;
            }
        }
        Ok(entries)
    }

    /// Resuelve una ruta absoluta (`/`, `/foo`, `/foo/bar`) a un inodo,
    /// caminando desde la raíz (inodo 2) componente a componente.
    pub fn resolve(&self, path: &str) -> Result<Inode, &'static str> {
        let mut current = self.read_inode(ROOT_INODE)?;
        for component in path.split('/').filter(|c| !c.is_empty()) {
            if !current.is_dir() {
                return Err("componente intermedio de la ruta no es un directorio");
            }
            let entries = self.read_dir(&current)?;
            let found = entries
                .iter()
                .find(|e| e.name == component)
                .ok_or("no existe en el ext2")?;
            current = self.read_inode(found.inode)?;
        }
        Ok(current)
    }
}
