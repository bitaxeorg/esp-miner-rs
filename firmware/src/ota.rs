//! Firmware images: the app partitions and what they hold, the running image's checksum, updates
//! written to the next OTA partition, and the choice of the partition to boot.

use bitcoin_hashes::{sha256, HashEngine};
use embedded_storage::nor_flash::NorFlash;
use esp_bootloader_esp_idf::{
    ota::OtaImageState,
    ota_updater::OtaUpdater,
    partitions::{
        read_partition_table, AppPartitionSubType, PartitionEntry, PartitionType,
        PARTITION_TABLE_MAX_LEN,
    },
};
use esp_storage::FlashStorage;
use heapless::{String, Vec};
use picoserve::io::Read;

use crate::{flash, wlog};

/// First byte of an ESP image, and magic word of its application descriptor.
const IMAGE_MAGIC: u8 = 0xE9;
const APP_DESC_MAGIC: u32 = 0xABCD_5432;
/// The descriptor follows the 24-byte image header and the 8-byte first segment header.
const APP_DESC_OFFSET: u32 = 32;
const SECTOR: usize = 4096;

pub struct Partition {
    pub label: String<16>,
    pub current: bool,
    pub factory: bool,
    /// From the image's application descriptor, when the partition holds an image.
    pub version: Option<String<32>>,
    pub compile_date: Option<String<16>>,
    pub compile_time: Option<String<16>>,
    /// Image size over partition size (%).
    pub usage_percent: Option<u8>,
}

pub struct Checksum {
    pub partition: String<16>,
    pub version: String<32>,
    pub size: u32,
    /// Lowercase hex.
    pub sha256: String<64>,
}

fn c_string<const N: usize>(bytes: &[u8]) -> String<N> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut s = String::new();
    for c in core::str::from_utf8(&bytes[..end]).unwrap_or("").chars() {
        if s.push(c).is_err() {
            break;
        }
    }
    s
}

/// Size of the image at `offset`: header, segments, checksum padding and the appended hash.
fn image_size(storage: &mut FlashStorage<'static>, offset: u32, limit: u32) -> Option<u32> {
    let mut header = [0u8; 24];
    embedded_storage::nor_flash::ReadNorFlash::read(storage, offset, &mut header).ok()?;
    if header[0] != IMAGE_MAGIC {
        return None;
    }
    let segments = header[1];
    let hash_appended = header[23] == 1;
    let mut position = 24u32;
    for _ in 0..segments {
        let mut segment = [0u8; 8];
        embedded_storage::nor_flash::ReadNorFlash::read(storage, offset + position, &mut segment)
            .ok()?;
        let len = u32::from_le_bytes([segment[4], segment[5], segment[6], segment[7]]);
        position = position.checked_add(8 + len)?;
        if position > limit {
            return None;
        }
    }
    // checksum byte at the end of the next 16-byte boundary
    let mut size = (position + 16) & !15;
    if hash_appended {
        size += 32;
    }
    (size <= limit).then_some(size)
}

fn describe(
    storage: &mut FlashStorage<'static>,
    entry: &PartitionEntry,
    booted: Option<u32>,
) -> Partition {
    let mut desc = [0u8; 256];
    let has_desc = embedded_storage::nor_flash::ReadNorFlash::read(
        storage,
        entry.offset() + APP_DESC_OFFSET,
        &mut desc,
    )
    .is_ok()
        && u32::from_le_bytes([desc[0], desc[1], desc[2], desc[3]]) == APP_DESC_MAGIC
        && image_size(storage, entry.offset(), entry.len()).is_some();
    let size = has_desc
        .then(|| image_size(storage, entry.offset(), entry.len()))
        .flatten();
    Partition {
        label: c_string(entry.label_as_str().as_bytes()),
        current: booted == Some(entry.offset()),
        factory: matches!(
            entry.partition_type(),
            PartitionType::App(AppPartitionSubType::Factory)
        ),
        version: has_desc.then(|| c_string(&desc[16..48])),
        compile_time: has_desc.then(|| c_string(&desc[80..96])),
        compile_date: has_desc.then(|| c_string(&desc[96..112])),
        usage_percent: size.map(|s| (u64::from(s) * 100 / u64::from(entry.len().max(1))) as u8),
    }
}

/// The app partitions, in table order.
pub async fn partitions() -> Vec<Partition, 4> {
    let mut list = Vec::new();
    let mut guard = flash::lock().await;
    let Some(storage) = guard.as_mut() else {
        return list;
    };
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    let (booted, entries) = {
        let Ok(table) = read_partition_table(storage, &mut buffer) else {
            return list;
        };
        let booted = table.booted_partition().ok().flatten().map(|p| p.offset());
        let entries: Vec<PartitionEntry, 8> = table
            .iter()
            .filter(|e| matches!(e.partition_type(), PartitionType::App(_)))
            .take(8)
            .collect();
        (booted, entries)
    };
    for entry in &entries {
        let _ = list.push(describe(storage, entry, booted));
    }
    list
}

/// The SHA-256 of the running image.
pub async fn checksum() -> Option<Checksum> {
    let mut guard = flash::lock().await;
    let storage = guard.as_mut()?;
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    let entry = read_partition_table(storage, &mut buffer)
        .ok()?
        .booted_partition()
        .ok()
        .flatten()?;
    let info = describe(storage, &entry, Some(entry.offset()));
    let size = image_size(storage, entry.offset(), entry.len())?;
    let mut engine = sha256::HashEngine::default();
    let mut chunk = [0u8; 1024];
    let mut done = 0;
    while done < size {
        let n = (size - done).min(chunk.len() as u32) as usize;
        embedded_storage::nor_flash::ReadNorFlash::read(
            storage,
            entry.offset() + done,
            &mut chunk[..n],
        )
        .ok()?;
        engine.input(&chunk[..n]);
        done += n as u32;
    }
    let hash = sha256::Hash::from_engine(engine);
    let mut hex = String::new();
    for b in hash.to_byte_array() {
        let _ = core::fmt::write(&mut hex, format_args!("{b:02x}"));
    }
    Some(Checksum {
        partition: info.label,
        version: info.version.unwrap_or_default(),
        size,
        sha256: hex,
    })
}

/// Marks the running image as good, so that a bootloader with rollback keeps it.
pub async fn confirm() {
    let mut guard = flash::lock().await;
    let Some(storage) = guard.as_mut() else {
        return;
    };
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    if let Ok(mut updater) = OtaUpdater::new(storage, &mut buffer) {
        if matches!(
            updater.current_ota_state(),
            Ok(OtaImageState::New | OtaImageState::PendingVerify)
        ) {
            let _ = updater.set_current_ota_state(OtaImageState::Valid);
        }
    }
}

/// Writes the image read from `body` to the next OTA partition and makes it the one to boot.
pub async fn update<R: Read>(body: &mut R) -> Result<(), &'static str> {
    let mut guard = flash::lock().await;
    let storage = guard.as_mut().ok_or("Flash unavailable")?;
    // on the heap: the buffers live across the reads of the body, and only during an update
    let mut table = alloc::vec![0u8; PARTITION_TABLE_MAX_LEN];
    let table: &mut [u8; PARTITION_TABLE_MAX_LEN] = table
        .as_mut_slice()
        .try_into()
        .map_err(|_| "Protocol Error")?;
    let mut updater = OtaUpdater::new(storage, table).map_err(|_| "No OTA partition")?;
    let written = write_image(&mut updater, body).await?;
    updater
        .activate_next_partition()
        .map_err(|_| "Validation / Activation Error")?;
    let _ = updater.set_current_ota_state(OtaImageState::New);
    wlog!(Info, "ota", "update written: {} bytes", written);
    Ok(())
}

/// Writes the image of `body` to the next OTA partition, a sector at a time. Returns its size.
async fn write_image<R: Read>(
    updater: &mut OtaUpdater<'_, 'static>,
    body: &mut R,
) -> Result<usize, &'static str> {
    let (mut region, slot) = updater.next_partition().map_err(|_| "No OTA partition")?;
    let capacity = region.partition_size();
    let mut nor = region.as_nor_flash().map_err(|_| "Protocol Error")?;
    wlog!(
        Info,
        "ota",
        "writing the update to {:?}",
        defmt::Debug2Format(&slot)
    );
    let mut sector = alloc::vec![0xFFu8; SECTOR];
    let mut written = 0usize;
    let mut first = true;
    loop {
        let mut filled = 0;
        while filled < SECTOR {
            let n = body
                .read(&mut sector[filled..])
                .await
                .map_err(|_| "Protocol Error")?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        if first {
            if sector[0] != IMAGE_MAGIC {
                return Err("Not a firmware image");
            }
            first = false;
        }
        if written + filled > capacity {
            return Err("File provided is too large for device");
        }
        sector[filled..].fill(0xFF);
        let offset = written as u32;
        nor.erase(offset, offset + SECTOR as u32)
            .map_err(|_| "Write Error")?;
        nor.write(offset, &sector).map_err(|_| "Write Error")?;
        written += filled;
        if filled < SECTOR {
            break;
        }
    }
    if written == 0 {
        return Err("Protocol Error");
    }
    Ok(written)
}

/// Makes the app partition `label` the one to boot.
pub async fn set_boot(label: &str) -> Result<(), &'static str> {
    let mut guard = flash::lock().await;
    let storage = guard.as_mut().ok_or("Flash unavailable")?;
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    let (subtype, offset, len) = {
        let table =
            read_partition_table(storage, &mut buffer).map_err(|_| "Partition not found")?;
        let entry = table
            .iter()
            .find(|e| e.label_as_str() == label)
            .ok_or("Partition not found")?;
        let PartitionType::App(subtype) = entry.partition_type() else {
            return Err("Partition not found");
        };
        (subtype, entry.offset(), entry.len())
    };
    if image_size(storage, offset, len).is_none() {
        return Err("No valid firmware found on partition");
    }
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    let mut updater = OtaUpdater::new(storage, &mut buffer).map_err(|_| "Partition not found")?;
    if subtype == AppPartitionSubType::Factory {
        updater.reset_data().map_err(|_| "Partition not found")?;
    } else {
        updater
            .ota_data()
            .and_then(|mut data| data.set_current_app_partition(subtype))
            .map_err(|_| "Partition not found")?;
    }
    Ok(())
}
