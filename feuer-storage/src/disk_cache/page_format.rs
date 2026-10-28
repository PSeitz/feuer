//! Experimental v15 metadata chunks. Each chunk contains 255 independently checksummed
//! entry-metadata pages and one checksummed next-chunk page. All-zero entry metadata is unused.
//! Payloads live in separate chunks. An entry's metadata never crosses a page boundary.

use twox_hash::XxHash64;

use super::metadata::EntryMetadata;
use crate::allocation::CHUNK_BYTES;

/// Size in bytes of a metadata page, including its header and padding.
/// Each page holds metadata for 101 entries (40 bytes each) and 24 padding bytes.
pub(super) const METADATA_PAGE_BYTES: usize = 4096;
pub(super) const PAGE_HEADER_BYTES: usize = 32;
pub(super) const ENTRY_METADATA_BYTES: usize = 40;
pub(super) const PAGE_CONTENT_BYTES: usize =
    (METADATA_PAGE_BYTES - PAGE_HEADER_BYTES) / ENTRY_METADATA_BYTES * ENTRY_METADATA_BYTES;
pub(super) const ENTRY_METADATA_PAGE_TAG: &[u8; 8] = b"FEUDES15";
pub(super) const NEXT_CHUNK_PAGE_TAG: &[u8; 8] = b"FEUNXT15";
/// Number of entries whose metadata fits in a page.
pub(super) const ENTRIES_PER_METADATA_PAGE: usize = PAGE_CONTENT_BYTES / ENTRY_METADATA_BYTES;
/// Number of entry-metadata pages in a chunk.
pub(super) const ENTRY_METADATA_PAGES_PER_CHUNK: usize = CHUNK_BYTES as usize / METADATA_PAGE_BYTES - 1;
/// Number of entries whose metadata fits in a chunk.
pub(super) const ENTRIES_PER_METADATA_CHUNK: usize = ENTRY_METADATA_PAGES_PER_CHUNK * ENTRIES_PER_METADATA_PAGE;
pub(super) const NO_CHUNK: u64 = u64::MAX;

/// The reserved content-checksum field is zero. Each page is validated independently.
pub(super) fn encode_page(page: &mut [u8], page_tag: &[u8; 8], entry_count: u64, contents: &[u8]) {
    assert_eq!(page.len(), METADATA_PAGE_BYTES);
    assert!(contents.len() <= PAGE_CONTENT_BYTES);
    page.fill(0);
    page[16..24].copy_from_slice(&entry_count.to_le_bytes());
    page[24..32].copy_from_slice(page_tag);
    page[PAGE_HEADER_BYTES..PAGE_HEADER_BYTES + contents.len()].copy_from_slice(contents);
    let checksum = XxHash64::oneshot(0, &page[8..]);
    page[..8].copy_from_slice(&checksum.to_le_bytes());
}

/// Validates a metadata page. The tag identifies the writer's format.
/// An intact checksum lets readers rely on its field, padding, and alignment guarantees.
pub(super) fn validate_page<'a>(page: &'a [u8], page_tag: &[u8; 8]) -> Option<&'a [u8]> {
    if page[24..32] != *page_tag || page[..8] != XxHash64::oneshot(0, &page[8..]).to_le_bytes() {
        return None;
    }
    Some(&page[PAGE_HEADER_BYTES..])
}

/// Key hash (u128), payload length, payload address, payload checksum (u64s).
/// All integers are little-endian. Aligned payload length is derived from payload length.
/// Payload checksums exclude alignment padding.
pub(super) fn encode_entry_metadata(bytes: &mut [u8; ENTRY_METADATA_BYTES], entry: &EntryMetadata) {
    bytes[..16].copy_from_slice(&entry.key.0.to_le_bytes());
    bytes[16..24].copy_from_slice(&entry.payload_len.to_le_bytes());
    bytes[24..32].copy_from_slice(&entry.payload_address.to_le_bytes());
    bytes[32..40].copy_from_slice(&entry.payload_checksum.to_le_bytes());
}
