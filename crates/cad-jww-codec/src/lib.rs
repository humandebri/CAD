//! Shared low-level JWW binary codec.

use encoding_rs::SHIFT_JIS;
use std::collections::BTreeMap;
use std::io::Cursor;
use thiserror::Error;

pub const JWW_SIGNATURE: &[u8; 8] = b"JwwData.";
pub const EXPORT_VERSION: u32 = 600;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("invalid JWW signature")]
    InvalidSignature,
    #[error("unexpected EOF while reading {0}")]
    UnexpectedEof(&'static str),
    #[error("JWW entity list was not found")]
    EntityListNotFound,
    #[error("unknown JWW class pid {0}")]
    UnknownClassPid(u32),
    #[error("unknown JWW entity class {0}")]
    UnknownEntityClass(String),
    #[error("invalid JWW block definition count {0}")]
    InvalidBlockDefinitionCount(u32),
    #[error("text cannot be represented in CP932: {0:?}")]
    UnencodableText(String),
    #[error("JWW record count exceeds 65534")]
    TooManyRecords,
    #[error("JWW class table exceeds the 16-bit archive limit")]
    ClassTableOverflow,
    #[error("invalid CP932 text")]
    InvalidTextEncoding,
    #[error("duplicate JWW block definition number {0}")]
    DuplicateBlockDefinition(u32),
}

pub type CodecResult<T> = Result<T, CodecError>;

pub struct Reader<'a> {
    cursor: Cursor<&'a [u8]>,
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            cursor: Cursor::new(data),
        }
    }

    #[must_use]
    pub fn bytes_read(&self) -> usize {
        self.cursor.position() as usize
    }

    pub fn skip(&mut self, len: usize) -> CodecResult<()> {
        let end = self
            .bytes_read()
            .checked_add(len)
            .ok_or(CodecError::UnexpectedEof("offset"))?;
        if end > self.cursor.get_ref().len() {
            return Err(CodecError::UnexpectedEof("bytes"));
        }
        self.cursor.set_position(end as u64);
        Ok(())
    }

    pub fn read_u8(&mut self) -> CodecResult<u8> {
        Ok(self.read_exact::<1>()?[0])
    }
    pub fn read_u16(&mut self) -> CodecResult<u16> {
        Ok(u16::from_le_bytes(self.read_exact::<2>()?))
    }
    pub fn read_u32(&mut self) -> CodecResult<u32> {
        Ok(u32::from_le_bytes(self.read_exact::<4>()?))
    }
    pub fn read_f64(&mut self) -> CodecResult<f64> {
        Ok(f64::from_le_bytes(self.read_exact::<8>()?))
    }

    pub fn read_bytes(&mut self, len: usize) -> CodecResult<Vec<u8>> {
        let end = self
            .bytes_read()
            .checked_add(len)
            .ok_or(CodecError::UnexpectedEof("offset"))?;
        if end > self.cursor.get_ref().len() {
            return Err(CodecError::UnexpectedEof("bytes"));
        }
        let mut value = vec![0; len];
        self.read_exact_into(&mut value)?;
        Ok(value)
    }

    pub fn read_cstring(&mut self) -> CodecResult<String> {
        let first = self.read_u8()?;
        let len = if first < 0xff {
            usize::from(first)
        } else {
            let word = self.read_u16()?;
            if word < 0xffff {
                usize::from(word)
            } else {
                self.read_u32()? as usize
            }
        };
        let bytes = self.read_bytes(len)?;
        let (decoded, _, had_errors) = SHIFT_JIS.decode(&bytes);
        if had_errors {
            return Err(CodecError::InvalidTextEncoding);
        }
        Ok(decoded.trim_end_matches('\0').to_owned())
    }

    fn read_exact<const N: usize>(&mut self) -> CodecResult<[u8; N]> {
        let mut value = [0; N];
        self.read_exact_into(&mut value)?;
        Ok(value)
    }

    fn read_exact_into(&mut self, value: &mut [u8]) -> CodecResult<()> {
        let start = self.bytes_read();
        let end = start
            .checked_add(value.len())
            .ok_or(CodecError::UnexpectedEof("offset"))?;
        let source = self.cursor.get_ref();
        if end > source.len() {
            return Err(CodecError::UnexpectedEof("bytes"));
        }
        value.copy_from_slice(&source[start..end]);
        self.cursor.set_position(end as u64);
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub name: String,
    pub state: u32,
    pub protect: u32,
}

#[derive(Debug, Clone)]
pub struct LayerGroup {
    pub name: String,
    pub state: u32,
    pub write_layer: u32,
    pub scale: f64,
    pub protect: u32,
    pub layers: [Layer; 16],
}

impl Default for LayerGroup {
    fn default() -> Self {
        Self {
            name: String::new(),
            state: 2,
            write_layer: 0,
            scale: 1.0,
            protect: 0,
            layers: std::array::from_fn(|_| Layer {
                state: 2,
                ..Layer::default()
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub memo: String,
    pub paper_size: u32,
    pub write_layer_group: u32,
    pub layer_groups: [LayerGroup; 16],
}

impl Default for Header {
    fn default() -> Self {
        Self {
            memo: "Exported by cadc".to_owned(),
            paper_size: 3,
            write_layer_group: 0,
            layer_groups: std::array::from_fn(|_| LayerGroup::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Base {
    pub group: u32,
    pub pen_style: u8,
    pub pen_color: u16,
    pub pen_width: u16,
    pub layer: u16,
    pub layer_group: u16,
    pub flag: u16,
}

#[derive(Debug, Clone)]
pub enum Record {
    Line {
        base: Base,
        p1: [f64; 2],
        p2: [f64; 2],
    },
    Arc {
        base: Base,
        center: [f64; 2],
        radius: f64,
        start_rad: f64,
        sweep_rad: f64,
        tilt_rad: f64,
        flatness: f64,
        full: bool,
    },
    Point {
        base: Base,
        at: [f64; 2],
        temporary: bool,
        marker: Option<(u32, f64, f64)>,
    },
    Text {
        base: Base,
        start: [f64; 2],
        end: [f64; 2],
        text_type: u32,
        size_x: f64,
        size_y: f64,
        spacing: f64,
        angle_deg: f64,
        font: String,
        value: String,
    },
    Dimension {
        base: Base,
        line: Box<Record>,
        text: Box<Record>,
    },
    Solid {
        base: Base,
        values: [f64; 8],
        color: Option<u32>,
    },
    Block {
        base: Base,
        ref_x: f64,
        ref_y: f64,
        scale_x: f64,
        scale_y: f64,
        rotation: f64,
        def_number: u32,
    },
}

impl Record {
    fn class_name(&self) -> &'static str {
        match self {
            Self::Line { .. } => "CDataSen",
            Self::Arc { .. } => "CDataEnko",
            Self::Point { .. } => "CDataTen",
            Self::Text { .. } => "CDataMoji",
            Self::Dimension { .. } => "CDataSunpou",
            Self::Solid { .. } => "CDataSolid",
            Self::Block { .. } => "CDataBlock",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub header: Header,
    pub records: Vec<Record>,
    pub blocks: Vec<BlockDefinition>,
}

#[derive(Debug, Clone)]
pub struct BlockDefinition {
    pub number: u32,
    pub name: String,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone)]
pub struct DecodedDocument {
    pub header: DecodedHeader,
    pub entities: Vec<DecodedEntity>,
    pub block_defs: Vec<DecodedBlockDefinition>,
}

#[derive(Debug, Clone)]
pub struct DecodedHeader {
    pub version: u32,
    pub memo: String,
    pub paper_size: u32,
    pub write_layer_group: u32,
    pub layer_groups: [DecodedLayerGroup; 16],
}

#[derive(Debug, Clone, Default)]
pub struct DecodedLayerGroup {
    pub layers: [DecodedLayer; 16],
    pub name: String,
    pub scale: f64,
    pub state: u32,
    pub write_layer: u32,
    pub protect: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DecodedLayer {
    pub name: String,
    pub state: u32,
    pub protect: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecodedBase {
    pub group: u32,
    pub pen_style: u8,
    pub pen_color: u16,
    pub pen_width: u16,
    pub layer: u16,
    pub layer_group: u16,
    pub flag: u16,
}

#[derive(Debug, Clone)]
pub enum DecodedEntity {
    Line(DecodedLine),
    Arc(DecodedArc),
    Point(DecodedPoint),
    Text(DecodedText),
    Solid(DecodedSolid),
    CircleSolid(DecodedCircleSolid),
    Block(DecodedBlock),
    Dimension(DecodedDimension),
}

#[derive(Debug, Clone)]
pub struct DecodedLine {
    pub base: DecodedBase,
    pub start: [f64; 2],
    pub end: [f64; 2],
}

#[derive(Debug, Clone)]
pub struct DecodedArc {
    pub base: DecodedBase,
    pub center: [f64; 2],
    pub radius: f64,
    pub start_rad: f64,
    pub sweep_rad: f64,
    pub tilt_rad: f64,
    pub flatness: f64,
    pub is_full_circle: bool,
}

#[derive(Debug, Clone)]
pub struct DecodedPoint {
    pub base: DecodedBase,
    pub at: [f64; 2],
    pub temporary: bool,
    pub marker_code: Option<u32>,
    pub rotation: f64,
    pub scale: f64,
}

#[derive(Debug, Clone)]
pub struct DecodedText {
    pub base: DecodedBase,
    pub start: [f64; 2],
    pub size_x: f64,
    pub size_y: f64,
    pub spacing: f64,
    pub angle: f64,
    pub mirror_y: bool,
    pub font_name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct DecodedSolid {
    pub base: DecodedBase,
    pub points: [[f64; 2]; 4],
    pub color: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DecodedCircleSolid {
    pub base: DecodedBase,
    pub center: [f64; 2],
    pub radius: f64,
    pub flatness: f64,
    pub tilt_rad: f64,
    pub start_rad: f64,
    pub sweep_rad: f64,
    pub solid_param: f64,
    pub color: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DecodedBlock {
    pub base: DecodedBase,
    pub ref_x: f64,
    pub ref_y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub rotation: f64,
    pub def_number: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedBlockDefinition {
    pub number: u32,
    pub name: String,
    pub entities: Vec<DecodedEntity>,
}

#[derive(Debug, Clone)]
pub struct DecodedDimension {
    pub base: DecodedBase,
    pub line: DecodedLine,
    pub text: DecodedText,
}

#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    pub fn u16(&mut self, value: u16) {
        self.bytes.extend(value.to_le_bytes());
    }
    pub fn u32(&mut self, value: u32) {
        self.bytes.extend(value.to_le_bytes());
    }
    pub fn f64(&mut self, value: f64) {
        self.bytes.extend(value.to_le_bytes());
    }
    pub fn raw(&mut self, value: &[u8]) {
        self.bytes.extend(value);
    }

    pub fn cstring(&mut self, value: &str) -> CodecResult<()> {
        let (encoded, _, had_errors) = SHIFT_JIS.encode(value);
        if had_errors {
            return Err(CodecError::UnencodableText(value.to_owned()));
        }
        let len = encoded.len();
        if len < 0xff {
            self.u8(len as u8);
        } else if len < 0xffff {
            self.u8(0xff);
            self.u16(len as u16);
        } else {
            self.u8(0xff);
            self.u16(0xffff);
            self.u32(len as u32);
        }
        self.raw(&encoded);
        Ok(())
    }
}

pub fn read_document(data: &[u8]) -> CodecResult<DecodedDocument> {
    let header = read_header(data)?;
    let entity_list_offset =
        find_entity_list_offset(data, header.version).ok_or(CodecError::EntityListNotFound)?;
    let mut reader = Reader::new(&data[entity_list_offset..]);
    let entities = read_entity_list(&mut reader, header.version)?;
    let block_data = &data[entity_list_offset + reader.bytes_read()..];
    let block_defs = if block_data.is_empty() || block_data == [0, 0] {
        Vec::new()
    } else {
        read_block_definitions(block_data, header.version)?
    };
    Ok(DecodedDocument {
        header,
        entities,
        block_defs,
    })
}

fn read_header(data: &[u8]) -> CodecResult<DecodedHeader> {
    if data.len() < JWW_SIGNATURE.len() || &data[..JWW_SIGNATURE.len()] != JWW_SIGNATURE {
        return Err(CodecError::InvalidSignature);
    }
    let mut reader = Reader::new(data);
    reader.skip(JWW_SIGNATURE.len())?;
    let version = reader.read_u32()?;
    let memo = reader.read_cstring()?;
    let paper_size = reader.read_u32()?;
    let write_layer_group = reader.read_u32()?;
    let mut layer_groups = std::array::from_fn(|_| DecodedLayerGroup {
        layers: std::array::from_fn(|_| DecodedLayer::default()),
        ..DecodedLayerGroup::default()
    });
    for group in &mut layer_groups {
        group.state = reader.read_u32()?;
        group.write_layer = reader.read_u32()?;
        group.scale = reader.read_f64()?;
        group.protect = reader.read_u32()?;
        for layer in &mut group.layers {
            layer.state = reader.read_u32()?;
            layer.protect = reader.read_u32()?;
        }
    }
    if version < 300 {
        apply_default_layer_names(&mut layer_groups);
    } else {
        match read_layer_names(&mut reader, version, &mut layer_groups) {
            Ok(()) => apply_default_layer_names_for_blanks(&mut layer_groups),
            Err(CodecError::UnexpectedEof(_)) => apply_default_layer_names(&mut layer_groups),
            Err(error) => return Err(error),
        }
    }
    Ok(DecodedHeader {
        version,
        memo,
        paper_size,
        write_layer_group,
        layer_groups,
    })
}

fn read_layer_names(
    reader: &mut Reader<'_>,
    version: u32,
    layer_groups: &mut [DecodedLayerGroup; 16],
) -> CodecResult<()> {
    if version < 300 {
        return Err(CodecError::UnexpectedEof("layer names"));
    }
    reader.skip((14 + 5 + 1 + 1) * 4)?;
    reader.skip(16 + 8 + 4 + 4 + 8 + 16 + 16)?;
    for group in layer_groups.iter_mut() {
        for layer in &mut group.layers {
            layer.name = reader.read_cstring()?;
        }
    }
    for group in layer_groups.iter_mut() {
        group.name = reader.read_cstring()?;
    }
    Ok(())
}

fn apply_default_layer_names(layer_groups: &mut [DecodedLayerGroup; 16]) {
    for (group_index, group) in layer_groups.iter_mut().enumerate() {
        group.name = format!("Group{group_index:X}");
        for (layer_index, layer) in group.layers.iter_mut().enumerate() {
            layer.name = format!("{group_index:X}-{layer_index:X}");
        }
    }
}

fn apply_default_layer_names_for_blanks(layer_groups: &mut [DecodedLayerGroup; 16]) {
    for (group_index, group) in layer_groups.iter_mut().enumerate() {
        if group.name.is_empty() {
            group.name = format!("Group{group_index:X}");
        }
        for (layer_index, layer) in group.layers.iter_mut().enumerate() {
            if layer.name.is_empty() {
                layer.name = format!("{group_index:X}-{layer_index:X}");
            }
        }
    }
}

fn find_entity_list_offset(data: &[u8], version: u32) -> Option<usize> {
    let expected_schema = version as u16;
    if data.len() < 128 {
        return None;
    }
    for index in 100..data.len().saturating_sub(20) {
        if data[index] != 0xff || data[index + 1] != 0xff {
            continue;
        }
        let schema = u16::from_le_bytes([data[index + 2], data[index + 3]]);
        let name_len = u16::from_le_bytes([data[index + 4], data[index + 5]]) as usize;
        if !(8..=32).contains(&name_len) || index + 6 + name_len > data.len() {
            continue;
        }
        let class_name = &data[index + 6..index + 6 + name_len];
        if !class_name.starts_with(b"CData") || index < 2 {
            continue;
        }
        let offset = index - 2;
        if schema == expected_schema {
            return Some(offset);
        }
    }
    None
}

pub fn read_entity_list(reader: &mut Reader<'_>, version: u32) -> CodecResult<Vec<DecodedEntity>> {
    let count = reader.read_u16()? as usize;
    let mut entities = Vec::with_capacity(count);
    let mut class_names = BTreeMap::<u32, String>::new();
    let mut next_pid = 1_u32;
    for _ in 0..count {
        let (entity, new_pid) = read_entity_with_pid(reader, version, &mut class_names, next_pid)?;
        next_pid = new_pid;
        if let Some(entity) = entity {
            entities.push(entity);
        }
    }
    Ok(entities)
}

fn read_entity_with_pid(
    reader: &mut Reader<'_>,
    version: u32,
    class_names: &mut BTreeMap<u32, String>,
    mut next_pid: u32,
) -> CodecResult<(Option<DecodedEntity>, u32)> {
    let class_id = reader.read_u16()?;
    let class_name = if class_id == 0xffff {
        let _schema_version = reader.read_u16()?;
        let name_len = reader.read_u16()? as usize;
        let name = String::from_utf8_lossy(&reader.read_bytes(name_len)?).into_owned();
        class_names.insert(next_pid, name.clone());
        next_pid += 1;
        name
    } else if class_id == 0x8000 {
        return Ok((None, next_pid));
    } else {
        let class_pid = u32::from(class_id & 0x7fff);
        class_names
            .get(&class_pid)
            .cloned()
            .ok_or(CodecError::UnknownClassPid(class_pid))?
    };
    let entity = match class_name.as_str() {
        "CDataSen" => DecodedEntity::Line(read_line(reader, version)?),
        "CDataEnko" => DecodedEntity::Arc(read_arc(reader, version)?),
        "CDataTen" => DecodedEntity::Point(read_point(reader, version)?),
        "CDataMoji" => DecodedEntity::Text(read_text(reader, version)?),
        "CDataSolid" => read_solid(reader, version)?,
        "CDataBlock" => DecodedEntity::Block(read_block(reader, version)?),
        "CDataSunpou" => DecodedEntity::Dimension(read_dimension(reader, version)?),
        _ => return Err(CodecError::UnknownEntityClass(class_name)),
    };
    next_pid += 1;
    Ok((Some(entity), next_pid))
}

fn read_base(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedBase> {
    Ok(DecodedBase {
        group: reader.read_u32()?,
        pen_style: reader.read_u8()?,
        pen_color: reader.read_u16()?,
        pen_width: if version >= 351 {
            reader.read_u16()?
        } else {
            0
        },
        layer: reader.read_u16()?,
        layer_group: reader.read_u16()?,
        flag: reader.read_u16()?,
    })
}

fn read_line(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedLine> {
    Ok(DecodedLine {
        base: read_base(reader, version)?,
        start: [reader.read_f64()?, reader.read_f64()?],
        end: [reader.read_f64()?, reader.read_f64()?],
    })
}

fn read_arc(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedArc> {
    Ok(DecodedArc {
        base: read_base(reader, version)?,
        center: [reader.read_f64()?, reader.read_f64()?],
        radius: reader.read_f64()?,
        start_rad: reader.read_f64()?,
        sweep_rad: reader.read_f64()?,
        tilt_rad: reader.read_f64()?,
        flatness: reader.read_f64()?,
        is_full_circle: reader.read_u32()? != 0,
    })
}

fn read_point(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedPoint> {
    let base = read_base(reader, version)?;
    let at = [reader.read_f64()?, reader.read_f64()?];
    let temporary = reader.read_u32()? != 0;
    let (marker_code, rotation, scale) = if base.pen_style == 100 {
        (
            Some(reader.read_u32()?),
            reader.read_f64()?,
            reader.read_f64()?,
        )
    } else {
        (None, 0.0, 1.0)
    };
    Ok(DecodedPoint {
        base,
        at,
        temporary,
        marker_code,
        rotation,
        scale,
    })
}

fn read_text(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedText> {
    let base = read_base(reader, version)?;
    let start = [reader.read_f64()?, reader.read_f64()?];
    let _end = [reader.read_f64()?, reader.read_f64()?];
    let _text_type = reader.read_u32()?;
    Ok(DecodedText {
        base,
        start,
        size_x: reader.read_f64()?,
        size_y: reader.read_f64()?,
        spacing: reader.read_f64()?,
        angle: reader.read_f64()?,
        mirror_y: false,
        font_name: reader.read_cstring()?,
        content: reader.read_cstring()?,
    })
}

fn read_solid(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedEntity> {
    let base = read_base(reader, version)?;
    let values = [
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
        reader.read_f64()?,
    ];
    let color = if base.pen_color == 10 {
        Some(reader.read_u32()?)
    } else {
        None
    };
    if base.pen_style >= 101 {
        Ok(DecodedEntity::CircleSolid(DecodedCircleSolid {
            base,
            center: [values[0], values[1]],
            radius: values[2],
            flatness: values[3],
            tilt_rad: values[4],
            start_rad: values[5],
            sweep_rad: values[6],
            solid_param: values[7],
            color,
        }))
    } else {
        Ok(DecodedEntity::Solid(DecodedSolid {
            base,
            points: [
                [values[0], values[1]],
                [values[4], values[5]],
                [values[6], values[7]],
                [values[2], values[3]],
            ],
            color,
        }))
    }
}

fn read_block(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedBlock> {
    Ok(DecodedBlock {
        base: read_base(reader, version)?,
        ref_x: reader.read_f64()?,
        ref_y: reader.read_f64()?,
        scale_x: reader.read_f64()?,
        scale_y: reader.read_f64()?,
        rotation: reader.read_f64()?,
        def_number: reader.read_u32()?,
    })
}

fn read_dimension(reader: &mut Reader<'_>, version: u32) -> CodecResult<DecodedDimension> {
    let base = read_base(reader, version)?;
    let line = read_line(reader, version)?;
    let text = read_text(reader, version)?;
    if version >= 420 {
        let _sxf_mode = reader.read_u16()?;
        for _ in 0..2 {
            let _aux_line = read_line(reader, version)?;
        }
        for _ in 0..4 {
            let _aux_point = read_point(reader, version)?;
        }
    }
    Ok(DecodedDimension { base, line, text })
}

pub fn read_block_definitions(
    data: &[u8],
    version: u32,
) -> CodecResult<Vec<DecodedBlockDefinition>> {
    let mut reader = Reader::new(data);
    let count = reader.read_u32()?;
    if count > 10_000 {
        return Err(CodecError::InvalidBlockDefinitionCount(count));
    }
    let mut definitions = Vec::with_capacity(count as usize);
    let mut numbers = std::collections::BTreeSet::new();
    let mut class_names = BTreeMap::<u32, String>::new();
    let mut next_pid = 1_u32;
    for _ in 0..count {
        let class_id = reader.read_u16()?;
        if class_id == 0xffff {
            let _schema = reader.read_u16()?;
            let name_len = reader.read_u16()? as usize;
            let class_name = String::from_utf8_lossy(&reader.read_bytes(name_len)?).into_owned();
            class_names.insert(next_pid, class_name);
            next_pid += 1;
        } else if class_id == 0x8000 {
            continue;
        } else {
            let class_pid = u32::from(class_id & 0x7fff);
            if !class_names.contains_key(&class_pid) {
                return Err(CodecError::UnknownClassPid(class_pid));
            }
        }
        let _base = read_base(&mut reader, version)?;
        let number = reader.read_u32()?;
        if !numbers.insert(number) {
            return Err(CodecError::DuplicateBlockDefinition(number));
        }
        let _is_referenced = reader.read_u32()?;
        reader.skip(4)?;
        let name = reader.read_cstring()?;
        let entities = read_entity_list(&mut reader, version)?;
        definitions.push(DecodedBlockDefinition {
            number,
            name,
            entities,
        });
    }
    Ok(definitions)
}

pub fn write_document(document: &Document) -> CodecResult<Vec<u8>> {
    if document.records.len() > 65_534 {
        return Err(CodecError::TooManyRecords);
    }
    let mut writer = Writer::default();
    write_header(&mut writer, &document.header)?;
    writer.u16(document.records.len() as u16);
    let mut archive = ArchiveWriter {
        writer,
        class_ids: BTreeMap::new(),
        position: 1,
    };
    for record in &document.records {
        archive.record(record)?;
    }
    write_block_definitions(&mut archive.writer, &document.blocks)?;
    Ok(archive.writer.into_bytes())
}

fn write_block_definitions(
    writer: &mut Writer,
    definitions: &[BlockDefinition],
) -> CodecResult<()> {
    writer.u32(definitions.len() as u32);
    for definition in definitions {
        writer.u16(0xffff);
        writer.u16(EXPORT_VERSION as u16);
        writer.u16(9);
        writer.raw(b"CDataList");
        write_base(writer, Base::default());
        writer.u32(definition.number);
        writer.u32(1);
        writer.u32(0);
        writer.cstring(&definition.name)?;
        if definition.records.len() > 65_534 {
            return Err(CodecError::TooManyRecords);
        }
        writer.u16(definition.records.len() as u16);
        let mut archive = ArchiveWriter {
            writer: std::mem::take(writer),
            class_ids: BTreeMap::new(),
            position: 1,
        };
        for record in &definition.records {
            archive.record(record)?;
        }
        *writer = archive.writer;
    }
    Ok(())
}

struct ArchiveWriter {
    writer: Writer,
    class_ids: BTreeMap<&'static str, u16>,
    position: u32,
}

impl ArchiveWriter {
    fn record(&mut self, record: &Record) -> CodecResult<()> {
        let class_name = record.class_name();
        if let Some(class_id) = self.class_ids.get(class_name).copied() {
            self.writer.u16(class_id | 0x8000);
        } else {
            let class_id =
                u16::try_from(self.position).map_err(|_| CodecError::ClassTableOverflow)?;
            self.class_ids.insert(class_name, class_id);
            self.position += 1;
            self.writer.u16(0xffff);
            self.writer.u16(EXPORT_VERSION as u16);
            self.writer.u16(class_name.len() as u16);
            self.writer.raw(class_name.as_bytes());
        }
        write_record_body(&mut self.writer, record)?;
        self.position += 1;
        Ok(())
    }
}

fn write_base(writer: &mut Writer, base: Base) {
    writer.u32(base.group);
    writer.u8(base.pen_style);
    writer.u16(base.pen_color);
    writer.u16(base.pen_width);
    writer.u16(base.layer);
    writer.u16(base.layer_group);
    writer.u16(base.flag);
}

fn write_record_body(writer: &mut Writer, record: &Record) -> CodecResult<()> {
    match record {
        Record::Line { base, p1, p2 } => {
            write_base(writer, *base);
            for value in [p1[0], p1[1], p2[0], p2[1]] {
                writer.f64(value);
            }
        }
        Record::Arc {
            base,
            center,
            radius,
            start_rad,
            sweep_rad,
            tilt_rad,
            flatness,
            full,
        } => {
            write_base(writer, *base);
            for value in [
                center[0], center[1], *radius, *start_rad, *sweep_rad, *tilt_rad, *flatness,
            ] {
                writer.f64(value);
            }
            writer.u32(u32::from(*full));
        }
        Record::Point {
            base,
            at,
            temporary,
            marker,
        } => {
            let mut base = *base;
            if marker.is_some() {
                base.pen_style = 100;
            }
            write_base(writer, base);
            writer.f64(at[0]);
            writer.f64(at[1]);
            writer.u32(u32::from(*temporary));
            if let Some((code, angle, scale)) = marker {
                writer.u32(*code);
                writer.f64(*angle);
                writer.f64(*scale);
            }
        }
        Record::Text {
            base,
            start,
            end,
            text_type,
            size_x,
            size_y,
            spacing,
            angle_deg,
            font,
            value,
        } => {
            write_base(writer, *base);
            for coordinate in [start[0], start[1], end[0], end[1]] {
                writer.f64(coordinate);
            }
            writer.u32(*text_type);
            for value in [*size_x, *size_y, *spacing, *angle_deg] {
                writer.f64(value);
            }
            writer.cstring(font)?;
            writer.cstring(value)?;
        }
        Record::Dimension { base, line, text } => {
            write_base(writer, *base);
            write_record_body(writer, line)?;
            write_record_body(writer, text)?;
            writer.u16(0);
            for _ in 0..2 {
                write_record_body(writer, &zero_line())?;
            }
            for _ in 0..4 {
                write_record_body(writer, &zero_point())?;
            }
        }
        Record::Solid {
            base,
            values,
            color,
        } => {
            write_base(writer, *base);
            for value in values {
                writer.f64(*value);
            }
            if base.pen_color == 10 {
                writer.u32(color.unwrap_or_default());
            }
        }
        Record::Block {
            base,
            ref_x,
            ref_y,
            scale_x,
            scale_y,
            rotation,
            def_number,
        } => {
            write_base(writer, *base);
            for value in [*ref_x, *ref_y, *scale_x, *scale_y, *rotation] {
                writer.f64(value);
            }
            writer.u32(*def_number);
        }
    }
    Ok(())
}

fn zero_line() -> Record {
    Record::Line {
        base: Base::default(),
        p1: [0.0, 0.0],
        p2: [0.0, 0.0],
    }
}
fn zero_point() -> Record {
    Record::Point {
        base: Base::default(),
        at: [0.0, 0.0],
        temporary: false,
        marker: None,
    }
}

fn write_header(writer: &mut Writer, header: &Header) -> CodecResult<()> {
    writer.raw(JWW_SIGNATURE);
    writer.u32(EXPORT_VERSION);
    writer.cstring(&header.memo)?;
    writer.u32(header.paper_size);
    writer.u32(header.write_layer_group);
    for group in &header.layer_groups {
        writer.u32(group.state);
        writer.u32(group.write_layer);
        writer.f64(group.scale);
        writer.u32(group.protect);
        for layer in &group.layers {
            writer.u32(layer.state);
            writer.u32(layer.protect);
        }
    }
    for _ in 0..14 {
        writer.u32(0);
    }
    for _ in 0..7 {
        writer.u32(0);
    }
    writer.f64(0.0);
    writer.f64(0.0);
    writer.f64(1.0);
    writer.u32(0);
    writer.u32(0);
    for _ in 0..5 {
        writer.f64(0.0);
    }
    for group in &header.layer_groups {
        for layer in &group.layers {
            writer.cstring(&layer.name)?;
        }
    }
    for group in &header.layer_groups {
        writer.cstring(&group.name)?;
    }
    writer.f64(0.0);
    writer.f64(0.0);
    writer.u32(0);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.u32(0);
    writer.f64(1.0);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.f64(1.0);
    writer.f64(0.0);
    writer.f64(0.0);
    for _ in 0..8 {
        writer.f64(0.0);
        writer.f64(0.0);
        writer.f64(0.0);
        writer.u32(0);
    }
    writer.f64(0.0);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.u32(0);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.u32(0);
    for _ in 0..11 {
        writer.f64(0.0);
    }
    for index in 0..10 {
        writer.u32(default_pen_rgb(index));
        writer.u32(1);
    }
    for index in 0..10 {
        writer.u32(default_pen_rgb(index));
        writer.u32(1);
        writer.f64(0.1);
    }
    for _ in 0..8 {
        for _ in 0..4 {
            writer.u32(0);
        }
    }
    for _ in 0..5 {
        for _ in 0..5 {
            writer.u32(0);
        }
    }
    for _ in 0..4 {
        for _ in 0..4 {
            writer.u32(0);
        }
    }
    for _ in 0..16 {
        writer.u32(0);
    }
    for _ in 0..9 {
        writer.f64(0.0);
    }
    writer.u32(0);
    writer.u32(0);
    for _ in 0..257 {
        writer.u32(0);
        writer.u32(1);
    }
    for _ in 0..257 {
        writer.cstring("")?;
        writer.u32(0);
        writer.u32(1);
        writer.f64(0.1);
    }
    for _ in 0..33 {
        for _ in 0..4 {
            writer.u32(0);
        }
    }
    for _ in 0..33 {
        writer.cstring("")?;
        writer.u32(0);
        for _ in 0..10 {
            writer.f64(0.0);
        }
    }
    for index in 1..=10 {
        writer.f64(2.5);
        writer.f64(2.5);
        writer.f64(0.0);
        writer.u32(index);
    }
    writer.f64(2.5);
    writer.f64(2.5);
    writer.f64(0.0);
    writer.u32(1);
    writer.u32(1);
    writer.f64(0.0);
    writer.f64(0.0);
    writer.u32(0);
    for _ in 0..6 {
        writer.f64(0.0);
    }
    Ok(())
}

fn default_pen_rgb(index: usize) -> u32 {
    const COLORS: [u32; 10] = [
        0, 0x00c0c0, 0xffffff, 0x00c000, 0xc0c000, 0xc000c0, 0xff0000, 0x808080, 0xff0080, 0xff80ff,
    ];
    COLORS[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_parseable_signature_and_header() {
        let bytes = write_document(&Document::default()).expect("header should encode");
        assert_eq!(&bytes[..8], JWW_SIGNATURE);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            EXPORT_VERSION
        );
        assert!(bytes.len() > 10_000);
    }

    #[test]
    fn reads_written_entity_and_block_lists() {
        let document = Document {
            records: vec![Record::Line {
                base: Base::default(),
                p1: [1.0, 2.0],
                p2: [3.0, 4.0],
            }],
            blocks: vec![BlockDefinition {
                number: 7,
                name: "PART".to_owned(),
                records: vec![Record::Point {
                    base: Base::default(),
                    at: [5.0, 6.0],
                    temporary: false,
                    marker: None,
                }],
            }],
            ..Document::default()
        };

        let bytes = write_document(&document).expect("document should encode");
        let decoded = read_document(&bytes).expect("document should decode");

        assert!(matches!(
            decoded.entities.as_slice(),
            [DecodedEntity::Line(_)]
        ));
        assert_eq!(decoded.block_defs.len(), 1);
        assert_eq!(decoded.block_defs[0].number, 7);
        assert_eq!(decoded.block_defs[0].name, "PART");
        assert!(matches!(
            decoded.block_defs[0].entities.as_slice(),
            [DecodedEntity::Point(_)]
        ));
    }

    #[test]
    fn rejects_unencodable_text() {
        let mut writer = Writer::default();
        assert!(matches!(
            writer.cstring("emoji 🚀"),
            Err(CodecError::UnencodableText(_))
        ));
    }
}
