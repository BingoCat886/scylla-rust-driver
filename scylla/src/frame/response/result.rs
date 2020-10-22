use anyhow::Result as AResult;
use std::str;
use byteorder::{BigEndian, ReadBytesExt};
use bytes::{Buf,Bytes};

use crate::frame::types;

#[derive(Debug)]
pub struct SetKeyspace {
    // TODO
}

#[derive(Debug)]
pub struct Prepared {
    pub id: Bytes,
    prepared_metadata: PreparedMetadata,
    metadata: ResultMetadata,
}

#[derive(Debug)]
pub struct SchemaChange {
    // TODO
}

#[derive(Clone,Debug)]
struct TableSpec {
    ks_name: String,
    table_name: String,
}

#[derive(Debug)]
struct PagingState {
    // TODO
}

#[derive(Debug)]
enum ColumnType {
    Ascii,
    Int,
    Text,
    Set(Box<ColumnType>),
    // TODO
}

#[derive(Debug)]
pub enum CQLValue {
    Ascii(String),
    Int(i32),
    Text(String),
    Set(Vec<CQLValue>),
    // TODO
}

impl CQLValue {
    pub fn as_ascii(&self) -> Option<&String> {
        match self {
            Self::Ascii(s) => Some(&s),
            _ => None
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None
        }
    }

    pub fn as_text(&self) -> Option<&String> {
        match self {
            Self::Text(s) => Some(&s),
            _ => None
        }
    }

    pub fn as_set(&self) -> Option<&Vec<CQLValue>> {
        match self {
            Self::Set(s) => Some(&s),
            _ => None
        }
    }

    // TODO
}

#[derive(Debug)]
struct ColumnSpec {
    table_spec: TableSpec,
    name: String,
    typ: ColumnType,
}

#[derive(Debug)]
struct ResultMetadata {
    col_count: usize,
    paging_state: Option<PagingState>,
    col_specs: Vec<ColumnSpec>
}

#[derive(Debug)]
struct PreparedMetadata {
    col_count: usize,
    pk_indexes: Vec<u16>,
    col_specs: Vec<ColumnSpec>
}

#[derive(Debug)]
pub struct Row {
    pub columns: Vec<Option<CQLValue>>,
}

#[derive(Debug)]
pub struct Rows {
    metadata: ResultMetadata,
    rows_count: usize,
    pub rows: Vec<Row>,
}

#[derive(Debug)]
pub enum Result {
    Void,
    Rows(Rows),
    SetKeyspace(SetKeyspace),
    Prepared(Prepared),
    SchemaChange(SchemaChange),
}

fn deser_table_spec(buf: &mut &[u8]) -> AResult<TableSpec> {
    let ks_name = types::read_string(buf)?.to_owned();
    let table_name = types::read_string(buf)?.to_owned();
    Ok(TableSpec{ ks_name, table_name })
}

fn deser_type(buf: &mut &[u8]) -> AResult<ColumnType> {
    use ColumnType::*;
    let id = types::read_short(buf)?;
    Ok(match id {
        0x0001 => Ascii,
        0x0009 => Int,
        0x000D => Text,
        0x0022 => Set(Box::new(deser_type(buf)?)),
        id => {
            // TODO implement other types
            return Err(anyhow!("Type not yet implemented, id: {}", id));
        }
    })
}

fn deser_col_specs(buf: &mut &[u8], global_table_spec: &Option<TableSpec>, col_count: usize) -> AResult<Vec<ColumnSpec>> {
    let mut col_specs = Vec::with_capacity(col_count);
    for _ in 0..col_count {
        let table_spec = if let Some(spec) = global_table_spec { spec.clone() } else { deser_table_spec(buf)? };
        let name = types::read_string(buf)?.to_owned();
        let typ = deser_type(buf)?;
        col_specs.push(ColumnSpec{ table_spec, name, typ });
    }
    Ok(col_specs)
}

fn deser_result_metadata(buf: &mut &[u8]) -> AResult<ResultMetadata> {
    let flags = types::read_int(buf)?;
    let global_tables_spec = flags & 0x0001 != 0;
    let has_more_pages = flags & 0x0002 != 0;
    let no_metadata = flags & 0x0004 != 0;

    let col_count = types::read_int(buf)?;
    if col_count < 0 {
        return Err(anyhow!("Invalid negative column count: {}", col_count));
    }
    let col_count = col_count as usize;

    assert!(!has_more_pages); // TODO handle paging
    let paging_state = None;

    if no_metadata {
        return Ok(ResultMetadata{col_count, paging_state, col_specs: vec![]});
    }

    let global_table_spec = if global_tables_spec { Some(deser_table_spec(buf)?) } else { None };

    let col_specs = deser_col_specs(buf, &global_table_spec, col_count)?;

    Ok(ResultMetadata{col_count, paging_state, col_specs})
}

fn deser_prepared_metadata(buf: &mut &[u8]) -> AResult<PreparedMetadata> {
    let flags = types::read_int(buf)?;
    let global_tables_spec = flags & 0x0001 != 0;

    let col_count = types::read_int(buf)?;
    if col_count < 0 {
        return Err(anyhow!("Invalid negative column count: {}", col_count));
    }
    let col_count = col_count as usize;

    let pk_count = types::read_int(buf)?;
    if pk_count < 0 {
        return Err(anyhow!("Invalid negative pk count: {}", col_count));
    }
    let pk_count = pk_count as usize;

    let mut pk_indexes = Vec::with_capacity(pk_count);
    for _ in 0..pk_count {
        pk_indexes.push(types::read_short(buf)? as u16);
    }

    let global_table_spec = if global_tables_spec { Some(deser_table_spec(buf)?) } else { None };

    let col_specs = deser_col_specs(buf, &global_table_spec, col_count)?;

    Ok(PreparedMetadata{col_count, pk_indexes, col_specs})
}

fn deser_cql_value(typ: &ColumnType, buf: &mut &[u8]) -> AResult<CQLValue> {
    use ColumnType::*;
    Ok(match typ {
        Ascii => {
            if !buf.is_ascii() {
                return Err(anyhow!("Not an ascii string: {:?}", buf));
            }
            CQLValue::Ascii(str::from_utf8(buf)?.to_owned())
        },
        Int => {
            if buf.len() != 4 {
                return Err(anyhow!("Expected buffer length of 4 bytes, got: {}", buf.len()));
            }
            CQLValue::Int(buf.read_i32::<BigEndian>()?)
        },
        Text => {
            CQLValue::Text(str::from_utf8(buf)?.to_owned())
        },
        Set(typ) => {
            let len = types::read_int(buf)?;
            if len < 0 {
                return Err(anyhow!("Invalid number of set elements: {}", len));
            }
            let mut res = Vec::with_capacity(len as usize);
            for _ in 0..len {
                // TODO: is `null` allowed as set element? Should we use read_bytes_opt?
                let mut b = types::read_bytes(buf)?;
                res.push(deser_cql_value(typ, &mut b)?);
            }
            CQLValue::Set(res)
        }
    })
}

fn deser_rows(buf: &mut &[u8]) -> AResult<Rows> {
    let metadata = deser_result_metadata(buf)?;

    // TODO: the protocol allows an optimization (which must be explicitly requested on query by
    // the driver) where the column metadata is not sent with the result.
    // Implement this optimization. We'll then need to take the column types by a parameter.
    // Beware of races; our column types may be outdated.
    assert!(metadata.col_count == metadata.col_specs.len());

    let rows_count = types::read_int(buf)?;
    if rows_count < 0 {
        return Err(anyhow!("Invalid negative number of rows: {}", rows_count));
    }
    let rows_count = rows_count as usize;

    let mut rows = Vec::with_capacity(rows_count);
    for _ in 0..rows_count {
        let mut columns = Vec::with_capacity(metadata.col_count);
        for i in 0..metadata.col_count {
            let v = if let Some(mut b) = types::read_bytes_opt(buf)? {
                Some(deser_cql_value(&metadata.col_specs[i].typ, &mut b)?)
            } else { None };
            columns.push(v);
        }
        rows.push(Row{columns: columns});
    }
    Ok(Rows{metadata, rows_count, rows})
}

fn deser_set_keyspace(_buf: &mut &[u8]) -> AResult<SetKeyspace> {
    Ok(SetKeyspace{}) // TODO
}

fn deser_prepared(buf: &mut &[u8]) -> AResult<Prepared> {
    let id_len = types::read_short(buf)? as usize;
    let id: Bytes = buf[0..id_len].to_owned().into();
    buf.advance(id_len);
    let prepared_metadata = deser_prepared_metadata(buf)?;
    let metadata = deser_result_metadata(buf)?;
    Ok(Prepared{ id, prepared_metadata, metadata })
}

fn deser_schema_change(_buf: &mut &[u8]) -> AResult<SchemaChange> {
    Ok(SchemaChange{}) // TODO
}

pub fn deserialize(buf: &mut &[u8]) -> AResult<Result> {
    use self::Result::*;
    Ok(match types::read_int(buf)? {
        0x0001 => Void,
        0x0002 => Rows(deser_rows(buf)?),
        0x0003 => SetKeyspace(deser_set_keyspace(buf)?),
        0x0004 => Prepared(deser_prepared(buf)?),
        0x0005 => SchemaChange(deser_schema_change(buf)?),
        k => return Err(anyhow!("Unknown query result kind: {}", k))
    })
}
