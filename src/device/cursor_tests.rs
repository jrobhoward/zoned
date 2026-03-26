#![allow(clippy::unwrap_used)]
#![allow(non_snake_case)]

use std::io::{Read, Seek, SeekFrom, Write};

use crate::types::Sector;

use super::*;

fn open_writable_tempfile() -> (ZonedDevice, tempfile::NamedTempFile) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    // Pre-fill with 8 KiB so reads return data
    std::fs::write(tmp.path(), [0u8; 8192]).unwrap();
    let dev = ZonedDevice::open_writable(tmp.path()).unwrap();
    (dev, tmp)
}

#[test]
fn cursor____new____starts_at_zero() {
    let (dev, _tmp) = open_writable_tempfile();
    let cursor = ZonedDeviceCursor::new(&dev);
    assert_eq!(cursor.position(), 0);
}

#[test]
fn cursor____at_sector____positions_correctly() {
    let (dev, _tmp) = open_writable_tempfile();
    let cursor = ZonedDeviceCursor::at_sector(&dev, Sector::new(4));
    assert_eq!(cursor.position(), 2048);
}

#[test]
fn cursor____sector_position____aligned() {
    let (dev, _tmp) = open_writable_tempfile();
    let cursor = ZonedDeviceCursor::at_sector(&dev, Sector::new(2));
    assert_eq!(cursor.sector_position(), Some(Sector::new(2)));
}

#[test]
fn cursor____seek_start____works() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor();
    let pos = cursor.seek(SeekFrom::Start(1024)).unwrap();
    assert_eq!(pos, 1024);
    assert_eq!(cursor.position(), 1024);
}

#[test]
fn cursor____seek_current_forward____works() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor_at(Sector::new(2)); // byte 1024
    let pos = cursor.seek(SeekFrom::Current(512)).unwrap();
    assert_eq!(pos, 1536);
}

#[test]
fn cursor____seek_current_backward____works() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor_at(Sector::new(4)); // byte 2048
    let pos = cursor.seek(SeekFrom::Current(-1024)).unwrap();
    assert_eq!(pos, 1024);
}

#[test]
fn cursor____seek_end____returns_unsupported() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor();
    let result = cursor.seek(SeekFrom::End(0));
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Unsupported);
}

#[test]
fn cursor____seek____rejects_unaligned() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor();
    let result = cursor.seek(SeekFrom::Start(100));
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[test]
fn cursor____seek_before_start____returns_error() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor();
    let result = cursor.seek(SeekFrom::Current(-1));
    assert!(result.is_err());
}

#[test]
fn cursor____write_then_read____round_trip() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor();

    let data = [0xAB_u8; 512];
    let written = cursor.write(&data).unwrap();
    assert_eq!(written, 512);
    assert_eq!(cursor.position(), 512);

    // Seek back and read
    cursor.seek(SeekFrom::Start(0)).unwrap();
    let mut buf = [0u8; 512];
    let n = cursor.read(&mut buf).unwrap();
    assert!(n > 0);
    assert_eq!(buf[0], 0xAB);
}

#[test]
fn cursor____flush____calls_fsync() {
    let (dev, _tmp) = open_writable_tempfile();
    let mut cursor = dev.cursor();
    // Flush should succeed (it calls fsync on a regular file)
    cursor.flush().unwrap();
}

#[test]
fn cursor____debug____shows_path_and_position() {
    let (dev, _tmp) = open_writable_tempfile();
    let cursor = dev.cursor_at(Sector::new(2));
    let debug = format!("{cursor:?}");
    assert!(debug.contains("ZonedDeviceCursor"));
    assert!(debug.contains("1024"));
}
