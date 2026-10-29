//! Persistent starts/runtime counters. Stored at a fixed offset inside the
//! SoC's default NVS data partition (located via
//! `esp-bootloader-esp-idf`'s partition table, so no custom
//! `partitions.csv` is needed) — NOT ESP-IDF's actual NVS key-value
//! format, just borrowing that partition's reserved flash space for our
//! own small record. See CLAUDE.md's "Persistence" section for why no
//! ring-buffer wear-leveling was needed here.

use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions;
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;

const MAGIC_V1: u32 = 0x5343_5f31; // "SC_1": starts + runtime only
const MAGIC: u32 = 0x5343_5f32; // "SC_2": + operator setpoint

/// Everything persisted across power cycles. Despite the name, also carries
/// the operator's PID setpoint (HMI nSetPoint) — one record, one flash
/// write, rather than a second store for a single float.
#[derive(Clone, Copy, Debug)]
pub struct Counters {
    pub total_starts: u32,
    pub total_runtime_s: u64,
    pub setpoint_bar: f32,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            total_starts: 0,
            total_runtime_s: 0,
            setpoint_bar: crate::config::P_MODULATE_START_BAR,
        }
    }
}

impl Counters {
    const RECORD_LEN: usize = 4 + 4 + 8 + 4 + 4; // magic + starts + runtime_s + setpoint + crc32
    const RECORD_LEN_V1: usize = 4 + 4 + 8 + 4; // magic + starts + runtime_s + crc32

    fn to_bytes(self) -> [u8; Self::RECORD_LEN] {
        let mut buf = [0u8; Self::RECORD_LEN];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4..8].copy_from_slice(&self.total_starts.to_le_bytes());
        buf[8..16].copy_from_slice(&self.total_runtime_s.to_le_bytes());
        buf[16..20].copy_from_slice(&self.setpoint_bar.to_le_bytes());
        let crc = crc32(&buf[0..20]);
        buf[20..24].copy_from_slice(&crc.to_le_bytes());
        buf
    }

    /// Also accepts a v1 record (written before the setpoint was
    /// persisted), keeping its counters and defaulting the setpoint.
    fn from_bytes(buf: &[u8; Self::RECORD_LEN]) -> Option<Self> {
        let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let (data_len, setpoint_bar) = match magic {
            MAGIC => (20, f32::from_le_bytes(buf[16..20].try_into().unwrap())),
            MAGIC_V1 => (Self::RECORD_LEN_V1 - 4, Self::default().setpoint_bar),
            _ => return None,
        };
        let crc = u32::from_le_bytes(buf[data_len..data_len + 4].try_into().unwrap());
        if crc32(&buf[0..data_len]) != crc {
            return None;
        }
        Some(Self {
            total_starts: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
            total_runtime_s: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
            setpoint_bar,
        })
    }
}

pub struct CounterStore {
    flash: FlashStorage<'static>,
    nvs_offset: u32,
}

impl CounterStore {
    /// Panics if the partition table can't be read or has no NVS data
    /// partition — both indicate a broken/missing partition table, which
    /// is unrecoverable at runtime.
    pub fn new(flash_peripheral: FLASH<'static>) -> Self {
        let mut flash = FlashStorage::new(flash_peripheral);

        let mut pt_mem = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
        let pt = partitions::read_partition_table(&mut flash, &mut pt_mem)
            .expect("failed to read partition table");
        let nvs = pt
            .find_partition(partitions::PartitionType::Data(
                partitions::DataPartitionSubType::Nvs,
            ))
            .expect("partition table read error")
            .expect("no NVS data partition in the partition table");
        let nvs_offset = nvs.offset();

        Self { flash, nvs_offset }
    }

    /// Returns zeroed counters if nothing valid has been persisted yet
    /// (first boot, or a torn/corrupt write from a power loss mid-write).
    pub fn load(&mut self) -> Counters {
        let mut buf = [0u8; Counters::RECORD_LEN];
        match self.flash.read(self.nvs_offset, &mut buf) {
            Ok(()) => Counters::from_bytes(&buf).unwrap_or_default(),
            Err(_) => Counters::default(),
        }
    }

    /// Best-effort — a failed flash write here shouldn't take down the
    /// controller; it just means this persist is lost, not fatal to the
    /// running state.
    pub fn save(&mut self, counters: Counters) {
        let buf = counters.to_bytes();
        let _ = self.flash.write(self.nvs_offset, &buf);
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}
