//! Minimal EXIF reader for JPEG and TIFF files: walks the JPEG marker
//! segments to APP1, then the TIFF IFD for the tags a photo sorter needs.
//! No dependencies, and it stops at the first thing it understands.

use crate::datetime::DateTime;

const TAG_DATETIME: u16 = 0x0132; // IFD0 DateTime
const TAG_EXIF_IFD: u16 = 0x8769; // pointer to the Exif sub-IFD
const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
const TAG_DATETIME_DIGITIZED: u16 = 0x9004;
const TAG_MAKE: u16 = 0x010F;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exif {
    pub date_time: Option<DateTime>,
    pub date_original: Option<DateTime>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub orientation: Option<u16>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExifError {
    NotAnImage,
    NoExifSegment,
    Malformed(&'static str),
}

impl std::fmt::Display for ExifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExifError::NotAnImage => write!(f, "not a JPEG or TIFF file"),
            ExifError::NoExifSegment => write!(f, "no EXIF segment found"),
            ExifError::Malformed(why) => write!(f, "malformed EXIF: {why}"),
        }
    }
}

impl std::error::Error for ExifError {}

/// Read EXIF from JPEG or TIFF bytes.
pub fn read_exif(data: &[u8]) -> Result<Exif, ExifError> {
    if data.len() < 4 {
        return Err(ExifError::NotAnImage);
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        read_from_jpeg(data)
    } else if data.starts_with(b"II") || data.starts_with(b"MM") {
        read_tiff(data)
    } else {
        Err(ExifError::NotAnImage)
    }
}

fn read_from_jpeg(data: &[u8]) -> Result<Exif, ExifError> {
    let mut offset = 2; // skip SOI
    while offset + 4 <= data.len() {
        if data[offset] != 0xFF {
            offset += 1;
            continue;
        }
        let marker = data[offset + 1];
        // standalone markers carry no length
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            offset += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break; // start of scan / end of image
        }
        let length = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        if length < 2 || offset + 2 + length > data.len() {
            return Err(ExifError::Malformed("segment length"));
        }
        if marker == 0xE1 {
            let segment = &data[offset + 4..offset + 2 + length];
            if segment.starts_with(b"Exif\0\0") {
                return read_tiff(&segment[6..]);
            }
        }
        offset += 2 + length;
    }
    Err(ExifError::NoExifSegment)
}

struct Tiff<'a> {
    data: &'a [u8],
    little_endian: bool,
}

impl<'a> Tiff<'a> {
    fn u16_at(&self, offset: usize) -> Result<u16, ExifError> {
        if offset + 2 > self.data.len() {
            return Err(ExifError::Malformed("u16 out of range"));
        }
        let bytes = [self.data[offset], self.data[offset + 1]];
        Ok(if self.little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    fn u32_at(&self, offset: usize) -> Result<u32, ExifError> {
        if offset + 4 > self.data.len() {
            return Err(ExifError::Malformed("u32 out of range"));
        }
        let bytes = [
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
        ];
        Ok(if self.little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    fn slice(&self, offset: usize, len: usize) -> Result<&'a [u8], ExifError> {
        if offset + len > self.data.len() {
            return Err(ExifError::Malformed("value out of range"));
        }
        Ok(&self.data[offset..offset + len])
    }
}

fn read_tiff(data: &[u8]) -> Result<Exif, ExifError> {
    if data.len() < 8 {
        return Err(ExifError::Malformed("TIFF header"));
    }
    let little_endian = match &data[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(ExifError::Malformed("TIFF byte order")),
    };
    let tiff = Tiff {
        data,
        little_endian,
    };
    if tiff.u16_at(2)? != 42 {
        return Err(ExifError::Malformed("TIFF magic"));
    }
    let ifd0 = tiff.u32_at(4)? as usize;
    let mut exif = Exif::default();
    let mut exif_ifd_offset: Option<usize> = None;

    read_ifd(&tiff, ifd0, &mut exif, &mut exif_ifd_offset)?;

    if let Some(offset) = exif_ifd_offset {
        // a broken sub-IFD must not sink the tags we already have
        let _ = read_ifd(&tiff, offset, &mut exif, &mut None);
    }
    Ok(exif)
}

fn read_ifd(
    tiff: &Tiff,
    start: usize,
    exif: &mut Exif,
    exif_ifd_offset: &mut Option<usize>,
) -> Result<(), ExifError> {
    let count = tiff.u16_at(start)? as usize;
    if count > 512 {
        return Err(ExifError::Malformed("implausible entry count"));
    }
    for i in 0..count {
        let entry = start + 2 + i * 12;
        let tag = tiff.u16_at(entry)?;
        let field_type = tiff.u16_at(entry + 2)?;
        let components = tiff.u32_at(entry + 4)? as usize;
        let value_offset = entry + 8;
        match tag {
            TAG_DATETIME => {
                if let Some(text) = read_ascii(tiff, value_offset, field_type, components)? {
                    exif.date_time = DateTime::parse_exif(&text);
                }
            }
            TAG_MAKE => exif.make = read_ascii(tiff, value_offset, field_type, components)?,
            TAG_MODEL => exif.model = read_ascii(tiff, value_offset, field_type, components)?,
            TAG_ORIENTATION => {
                if field_type == 3 {
                    exif.orientation = Some(tiff.u16_at(value_offset)?);
                }
            }
            TAG_EXIF_IFD => {
                if field_type == 4 {
                    *exif_ifd_offset = Some(tiff.u32_at(value_offset)? as usize);
                }
            }
            TAG_DATETIME_ORIGINAL => {
                if let Some(text) = read_ascii(tiff, value_offset, field_type, components)? {
                    exif.date_original = DateTime::parse_exif(&text);
                }
            }
            TAG_DATETIME_DIGITIZED => {
                if let Some(text) = read_ascii(tiff, value_offset, field_type, components)? {
                    if exif.date_original.is_none() {
                        exif.date_original = DateTime::parse_exif(&text);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// ASCII values longer than four bytes live at an offset; shorter ones sit
/// inline in the entry's value field.
fn read_ascii(
    tiff: &Tiff,
    value_offset: usize,
    field_type: u16,
    components: usize,
) -> Result<Option<String>, ExifError> {
    if field_type != 2 && field_type != 1 && field_type != 7 {
        return Ok(None);
    }
    let start = if components <= 4 {
        value_offset
    } else {
        tiff.u32_at(value_offset)? as usize
    };
    if start + components > tiff.data.len() {
        return Err(ExifError::Malformed("string out of range"));
    }
    let bytes = tiff.slice(start, components)?;
    let text: String = bytes
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as char)
        .collect();
    Ok(Some(text.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal JPEG carrying an APP1 EXIF segment (test fixture).
    pub fn build_jpeg_with_exif(tiff_payload: &[u8]) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8]; // SOI
        let mut app1 = vec![0xFF, 0xE1];
        let length = (2 + 6 + tiff_payload.len()) as u16;
        app1.extend_from_slice(&length.to_be_bytes());
        app1.extend_from_slice(b"Exif\0\0");
        app1.extend_from_slice(tiff_payload);
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]); // SOS
        jpeg
    }

    /// Build a little-endian TIFF block with IFD0 entries. Values longer
    /// than four bytes are written after the directory, which is the
    /// layout the real format uses.
    pub fn build_tiff(entries: &[(u16, u16, u32, &[u8])]) -> Vec<u8> {
        let ifd_offset = 8u32;
        let data_start = ifd_offset + 2 + entries.len() as u32 * 12 + 4;

        let mut directory: Vec<u8> = Vec::new();
        directory.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let mut data_area: Vec<u8> = Vec::new();
        for (tag, field_type, count, value) in entries {
            directory.extend_from_slice(&tag.to_le_bytes());
            directory.extend_from_slice(&field_type.to_le_bytes());
            directory.extend_from_slice(&count.to_le_bytes());
            if value.len() <= 4 {
                let mut padded = value.to_vec();
                padded.resize(4, 0);
                directory.extend_from_slice(&padded);
            } else {
                let offset = data_start + data_area.len() as u32;
                directory.extend_from_slice(&offset.to_le_bytes());
                data_area.extend_from_slice(value);
            }
        }
        directory.extend_from_slice(&0u32.to_le_bytes()); // no IFD1

        let mut out = Vec::new();
        out.extend_from_slice(b"II");
        out.extend_from_slice(&42u16.to_le_bytes());
        out.extend_from_slice(&ifd_offset.to_le_bytes());
        out.extend_from_slice(&directory);
        out.extend_from_slice(&data_area);
        out
    }

    #[test]
    fn rejects_non_images() {
        assert_eq!(read_exif(b"hello world"), Err(ExifError::NotAnImage));
        assert_eq!(read_exif(b"\x00\x01"), Err(ExifError::NotAnImage));
    }

    #[test]
    fn jpeg_without_exif_reports_cleanly() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(read_exif(&jpeg), Err(ExifError::NoExifSegment));
    }

    #[test]
    fn reads_date_time_from_ifd0() {
        let tiff = build_tiff(&[(TAG_DATETIME, 2, 20, b"2026:09:27 12:34:56\0")]);
        let jpeg = build_jpeg_with_exif(&tiff);
        let exif = read_exif(&jpeg).unwrap();
        let when = exif.date_time.expect("date");
        assert_eq!((when.year, when.month, when.day), (2026, 9, 27));
        assert_eq!((when.hour, when.minute, when.second), (12, 34, 56));
    }

    #[test]
    fn reads_make_and_model() {
        let tiff = build_tiff(&[
            (TAG_MAKE, 2, 9, b"Fujifilm\0"),
            (TAG_MODEL, 2, 5, b"X-T5\0"),
        ]);
        let exif = read_exif(&tiff).unwrap();
        assert_eq!(exif.make.as_deref(), Some("Fujifilm"));
        assert_eq!(exif.model.as_deref(), Some("X-T5"));
    }

    #[test]
    fn short_ascii_values_are_inline() {
        // three bytes fit inside the entry's value field
        let tiff = build_tiff(&[(TAG_MAKE, 2, 3, b"HP\0")]);
        let exif = read_exif(&tiff).unwrap();
        assert_eq!(exif.make.as_deref(), Some("HP"));
    }

    #[test]
    fn big_endian_tiff() {
        let mut out = Vec::new();
        out.extend_from_slice(b"MM");
        out.extend_from_slice(&42u16.to_be_bytes());
        out.extend_from_slice(&8u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&TAG_DATETIME.to_be_bytes());
        out.extend_from_slice(&2u16.to_be_bytes()); // ASCII
        out.extend_from_slice(&20u32.to_be_bytes());
        let value_offset = out.len() as u32 + 4; // the entry is 12 bytes
        out.extend_from_slice(&value_offset.to_be_bytes());
        out.extend_from_slice(b"2020:01:02 03:04:05\0");
        let exif = read_exif(&out).unwrap();
        let when = exif.date_time.unwrap();
        assert_eq!((when.year, when.month, when.day), (2020, 1, 2));
    }

    #[test]
    fn truncated_files_fail_cleanly() {
        // segment header promises 16 bytes that are not there
        assert!(matches!(
            read_exif(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x10]),
            Err(ExifError::Malformed(_))
        ));
        assert!(matches!(
            read_exif(b"II\x2a\x00\x08\x00\x00\x00"),
            Err(ExifError::Malformed(_))
        ));
    }
}
