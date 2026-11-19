//! The SPI flash, shared by the settings (NVS partition) and the firmware updates (OTA
//! partitions), and the partition table.

use core::cell::Cell;

use defmt::{error, info};
use embassy_sync::{
    blocking_mutex::{raw::CriticalSectionRawMutex, Mutex as BlockingMutex},
    mutex::{Mutex, MutexGuard},
};
use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};
use esp_bootloader_esp_idf::partitions::{
    read_partition_table, DataPartitionSubType, PartitionType, PARTITION_TABLE_MAX_LEN,
};
use esp_storage::{FlashStorage, FlashStorageError};

/// A partition: offset and size in bytes.
#[derive(Clone, Copy, Debug, defmt::Format)]
pub struct Region {
    pub offset: u32,
    pub size: u32,
}

static FLASH: Mutex<CriticalSectionRawMutex, Option<FlashStorage<'static>>> = Mutex::new(None);
static NVS: BlockingMutex<CriticalSectionRawMutex, Cell<Option<Region>>> =
    BlockingMutex::new(Cell::new(None));

/// Takes the flash and finds the NVS partition in the partition table.
pub async fn init(flash: esp_hal::peripherals::FLASH<'static>) {
    let mut storage = FlashStorage::new(flash).multicore_auto_park();
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    match read_partition_table(&mut storage, &mut buffer) {
        Ok(table) => match table.find_partition(PartitionType::Data(DataPartitionSubType::Nvs)) {
            Ok(Some(nvs)) => {
                let region = Region {
                    offset: nvs.offset(),
                    size: nvs.len(),
                };
                info!("NVS partition: {}", region);
                NVS.lock(|n| n.set(Some(region)));
            }
            _ => error!("no NVS partition: settings won't be kept"),
        },
        Err(e) => error!("partition table unreadable: {}", e),
    }
    *FLASH.lock().await = Some(storage);
}

/// The NVS partition, if the partition table has one.
pub fn nvs_partition() -> Option<Region> {
    NVS.lock(Cell::get)
}

/// The flash, for exclusive use while the guard lives. Panics before [`init`].
pub async fn lock() -> MutexGuard<'static, CriticalSectionRawMutex, Option<FlashStorage<'static>>> {
    FLASH.lock().await
}

/// The flash as the NVS library wants it: storage plus the ROM CRC32.
pub struct NvsFlash<'a>(pub &'a mut FlashStorage<'static>);

impl ErrorType for NvsFlash<'_> {
    type Error = FlashStorageError;
}

impl ReadNorFlash for NvsFlash<'_> {
    const READ_SIZE: usize = FlashStorage::READ_SIZE;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.0.read(offset, bytes)
    }

    fn capacity(&self) -> usize {
        self.0.capacity()
    }
}

impl NorFlash for NvsFlash<'_> {
    const WRITE_SIZE: usize = FlashStorage::WRITE_SIZE;
    const ERASE_SIZE: usize = FlashStorage::ERASE_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.0.erase(from, to)
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.0.write(offset, bytes)
    }
}

impl esp_nvs::platform::Crc for NvsFlash<'_> {
    fn crc32(init: u32, data: &[u8]) -> u32 {
        esp_hal::rom::crc::crc32_le(init, data)
    }
}
