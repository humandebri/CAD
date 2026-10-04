//! JWS header layout is independently described by its reader's author at
//! https://www.jwcad.net/log/consult2/consult2-0610-12/thread17752.htm.
//! Only layouts exercised by the official distribution corpus are enabled.
//! Unlike our generated JWW archives, JWS uses one MFC class table throughout
//! the symbol and its block definitions, and a WORD block-list count.

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolHeader {
    pub version: u32,
    pub origin: [f64; 2],
    pub group_scales: [f64; 16],
    /// Preserved for inspection; its interpretation is not established.
    pub reserved: Vec<u8>,
    pub bounds: [f64; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedSymbol {
    pub header: SymbolHeader,
    pub entities: Vec<DecodedEntity>,
    pub block_defs: Vec<DecodedBlockDefinition>,
}

/// Decodes a complete known JWS archive. Never scans for a guessed record
/// boundary or accepts an unexplained suffix.
pub fn read_symbol(data: &[u8]) -> CodecResult<DecodedSymbol> {
    let mut reader = Reader::new(data);
    if reader.read_bytes(8)? != b"JwsData." {
        return Err(CodecError::InvalidSymbolHeader("signature"));
    }
    if reader.read_bytes(192)?.iter().any(|byte| *byte != b'.') {
        return Err(CodecError::InvalidSymbolHeader("unrecognized prefix"));
    }
    let version = reader.read_u32()?;
    if ![351, 420, 600].contains(&version) {
        return Err(CodecError::UnsupportedSymbolVersion(version));
    }
    let origin = [reader.read_f64()?, reader.read_f64()?];
    let mut group_scales = [0.; 16];
    for scale in &mut group_scales {
        *scale = reader.read_f64()?;
    }
    let reserved = reader.read_bytes(72)?;
    let bounds = [
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
    ];
    if origin
        .iter()
        .chain(bounds.iter())
        .any(|value| !value.is_finite())
        || group_scales
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.)
        || bounds[0] > bounds[2]
        || bounds[1] > bounds[3]
    {
        return Err(CodecError::InvalidSymbolHeader(
            "invalid origin, scales or bounds",
        ));
    }
    let mut class_names = BTreeMap::new();
    let mut next_pid = 1;
    let entities = entity_list(&mut reader, version, &mut class_names, &mut next_pid)?;
    let count = u32::from(reader.read_u16()?);
    if count > 10_000 {
        return Err(CodecError::InvalidBlockDefinitionCount(count));
    }
    let mut block_defs = Vec::with_capacity(count as usize);
    let mut numbers = std::collections::BTreeSet::new();
    for _ in 0..count {
        let class_id = reader.read_u16()?;
        let class_name = if class_id == 0xffff {
            let schema = reader.read_u16()?;
            if u32::from(schema) != version {
                return Err(CodecError::InvalidSymbolHeader(
                    "unvalidated block class schema",
                ));
            }
            let length = usize::from(reader.read_u16()?);
            let name = String::from_utf8_lossy(&reader.read_bytes(length)?).into_owned();
            class_names.insert(next_pid, name.clone());
            next_pid += 1;
            name
        } else {
            let pid = u32::from(class_id & 0x7fff);
            class_names
                .get(&pid)
                .cloned()
                .ok_or(CodecError::UnknownClassPid(pid))?
        };
        if class_name != "CDataList" {
            return Err(CodecError::UnknownEntityClass(class_name));
        }
        // The CDataList object itself is registered before its nested archive.
        next_pid += 1;
        let base = read_base(&mut reader, version)?;
        let number = reader.read_u32()?;
        if !numbers.insert(number) {
            return Err(CodecError::DuplicateBlockDefinition(number));
        }
        let is_referenced = reader.read_u32()?;
        let reserved = reader.read_u32()?;
        let name = reader.read_cstring()?;
        let entities = entity_list(&mut reader, version, &mut class_names, &mut next_pid)?;
        block_defs.push(DecodedBlockDefinition {
            base,
            number,
            is_referenced,
            reserved,
            name,
            entities,
        });
    }
    if reader.bytes_read() != data.len() {
        return Err(CodecError::SymbolTrailingBytes(
            data.len() - reader.bytes_read(),
        ));
    }
    Ok(DecodedSymbol {
        header: SymbolHeader {
            version,
            origin,
            group_scales,
            reserved,
            bounds,
        },
        entities,
        block_defs,
    })
}

fn entity_list(
    reader: &mut Reader<'_>,
    version: u32,
    class_names: &mut BTreeMap<u32, String>,
    next_pid: &mut u32,
) -> CodecResult<Vec<DecodedEntity>> {
    let count = usize::from(reader.read_u16()?);
    let mut entities = Vec::with_capacity(count);
    for _ in 0..count {
        let start = reader.cursor.position();
        if reader.read_u16()? == 0xffff && u32::from(reader.read_u16()?) != version {
            return Err(CodecError::InvalidSymbolHeader(
                "unvalidated entity class schema",
            ));
        }
        reader.cursor.set_position(start);
        let (entity, next) = read_entity_with_pid(reader, version, class_names, *next_pid)?;
        *next_pid = next;
        if let Some(entity) = entity {
            entities.push(entity);
        }
    }
    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(version: u32) -> Writer {
        let mut writer = Writer::default();
        writer.raw(b"JwsData.");
        writer.raw(&[b'.'; 192]);
        writer.u32(version);
        writer.f64(1.);
        writer.f64(2.);
        for _ in 0..16 {
            writer.f64(100.);
        }
        writer.raw(&[0; 72]);
        for value in [0., 0., 10., 20.] {
            writer.f64(value);
        }
        writer
    }

    fn line(writer: &mut Writer, version: u32) {
        writer.u32(0);
        writer.u8(1);
        writer.u16(2);
        if version >= 351 {
            writer.u16(0);
        }
        writer.u16(0);
        writer.u16(0);
        writer.u16(0);
        for value in [0., 0., 10., 20.] {
            writer.f64(value);
        }
    }

    #[test]
    fn known_versions_and_complete_boundary() {
        for version in [351, 420, 600] {
            let mut writer = header(version);
            writer.u16(1);
            writer.u16(0xffff);
            writer.u16(version as u16);
            writer.u16(8);
            writer.raw(b"CDataSen");
            line(&mut writer, version);
            writer.u16(0);
            let mut data = writer.into_bytes();
            let decoded = read_symbol(&data).unwrap();
            assert_eq!(decoded.header.version, version);
            assert_eq!(decoded.entities.len(), 1);
            let mut unknown_schema = data.clone();
            unknown_schema[456..458].copy_from_slice(&1000u16.to_le_bytes());
            assert!(matches!(
                read_symbol(&unknown_schema),
                Err(CodecError::InvalidSymbolHeader(_))
            ));
            data.push(0);
            assert!(matches!(
                read_symbol(&data),
                Err(CodecError::SymbolTrailingBytes(1))
            ));
        }
    }

    #[test]
    fn shared_class_table_in_block_lists_and_unknown_classes() {
        let mut writer = header(600);
        writer.u16(1);
        writer.u16(0xffff);
        writer.u16(600);
        writer.u16(8);
        writer.raw(b"CDataSen");
        line(&mut writer, 600);
        writer.u16(2);
        for number in [4, 5] {
            if number == 4 {
                writer.u16(0xffff);
                writer.u16(600);
                writer.u16(9);
                writer.raw(b"CDataList");
            } else {
                writer.u16(0x8003);
            }
            write_base(&mut writer, DecodedBase::default().into());
            writer.u32(number);
            writer.u32(0);
            writer.u32(0);
            writer.cstring("日本語").unwrap();
            writer.u16(1);
            writer.u16(0x8001);
            line(&mut writer, 600);
        }
        let data = writer.into_bytes();
        let decoded = read_symbol(&data).unwrap();
        assert_eq!(decoded.block_defs.len(), 2);
        assert_eq!(decoded.block_defs[1].entities, decoded.entities);
        let mut unknown = data.clone();
        let offset = unknown.windows(9).position(|b| b == b"CDataList").unwrap();
        unknown[offset..offset + 9].copy_from_slice(b"CDataWhat");
        assert!(matches!(
            read_symbol(&unknown),
            Err(CodecError::UnknownEntityClass(_))
        ));
        for length in [0, 8, 199, 203, 300, data.len() - 1] {
            assert!(read_symbol(&data[..length]).is_err());
        }
    }

    #[test]
    fn rejects_unvalidated_versions_headers_and_invalid_scales() {
        let mut data = header(1000).into_bytes();
        data.extend([0; 4]);
        assert!(matches!(
            read_symbol(&data),
            Err(CodecError::UnsupportedSymbolVersion(1000))
        ));
        data[200..204].copy_from_slice(&600u32.to_le_bytes());
        data[220..228].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(matches!(
            read_symbol(&data),
            Err(CodecError::InvalidSymbolHeader(_))
        ));
        data[8] = 0;
        assert!(matches!(
            read_symbol(&data),
            Err(CodecError::InvalidSymbolHeader(_))
        ));
    }
}
