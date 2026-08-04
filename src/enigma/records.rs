//! Enigma VB VFS record parser.

use std::io::{Cursor, Read};

use crate::enigma::{Error, VfsEntry, VfsEntryKind};

const EVB_MAGIC: &[u8; 4] = b"EVB\x00";

const NODE_TYPE_FILE: u8 = 2;
const NODE_TYPE_FOLDER: u8 = 3;

const PACK_HEADER_SIZE: usize = 64;

const HEADER_NODE_SIZE: usize = 16;

#[derive(Debug)]
#[allow(dead_code)]
struct FlatNode {
    name: String,
    node_type: u8,
    objects_count: u32,
    offset: u64,
    stored_size: u32,
    original_size: u32,
}

pub fn parse_vfs_tree(data: &[u8]) -> Result<Vec<VfsEntry>, Error> {
    let mut cursor = Cursor::new(data);
    let flat_nodes = read_all_nodes(&mut cursor)?;
    if flat_nodes.is_empty() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let mut idx: usize = 1; // skip main node (index 0)
    let count = flat_nodes[0].objects_count as usize;
    walk_children(&flat_nodes, &mut idx, count, "", &mut entries);
    Ok(entries)
}

fn read_all_nodes(cursor: &mut Cursor<&[u8]>) -> Result<Vec<FlatNode>, Error> {
    let mut hdr_buf = [0u8; PACK_HEADER_SIZE];
    read_exact(cursor, &mut hdr_buf)?;
    if &hdr_buf[0..4] != EVB_MAGIC {
        return Err(Error::InvalidMagic);
    }

    let main_node = read_header_node(cursor)?;

    let mut abs_offset = cursor.position() + main_node.size as u64 - 12;

    let pos = cursor.position();
    if pos > 0 {
        cursor.set_position(pos - 1);
    }

    let mut nodes = vec![FlatNode {
        name: String::new(),
        node_type: 0, // main
        objects_count: main_node.objects_count,
        offset: 0,
        stored_size: 0,
        original_size: 0,
    }];

    loop {
        let header_node = match read_header_node(cursor) {
            Ok(h) => h,
            Err(Error::UnexpectedEof) => break,
            Err(e) => return Err(e),
        };

        let named = match read_named_node(cursor) {
            Ok(n) => n,
            Err(Error::UnexpectedEof) => break,
            Err(e) => return Err(e),
        };

        match named.node_type {
            NODE_TYPE_FILE => {
                let opt = read_optional_file_node(cursor)?;
                let offset = abs_offset;
                abs_offset += opt.stored_size as u64;
                nodes.push(FlatNode {
                    name: named.name,
                    node_type: NODE_TYPE_FILE,
                    objects_count: header_node.objects_count,
                    offset,
                    stored_size: opt.stored_size,
                    original_size: opt.original_size,
                });
            }
            NODE_TYPE_FOLDER => {
                let mut skip = [0u8; 25];
                read_exact(cursor, &mut skip)?;
                nodes.push(FlatNode {
                    name: named.name,
                    node_type: NODE_TYPE_FOLDER,
                    objects_count: header_node.objects_count,
                    offset: 0,
                    stored_size: 0,
                    original_size: 0,
                });
            }
            _ => {
                break;
            }
        }
    }

    Ok(nodes)
}

fn walk_children(
    nodes: &[FlatNode],
    idx: &mut usize,
    count: usize,
    prefix: &str,
    entries: &mut Vec<VfsEntry>,
) {
    for _ in 0..count {
        if *idx >= nodes.len() {
            break;
        }
        let node = &nodes[*idx];
        *idx += 1;

        let name = normalize_folder_name(&node.name);
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };

        match node.node_type {
            NODE_TYPE_FILE => {
                entries.push(VfsEntry {
                    path,
                    kind: VfsEntryKind::File,
                    offset: node.offset,
                    stored_size: node.stored_size,
                    original_size: node.original_size,
                });
            }
            NODE_TYPE_FOLDER => {
                let child_count = node.objects_count as usize;
                walk_children(nodes, idx, child_count, &path, entries);
            }
            _ => {}
        }
    }
}

fn normalize_folder_name(name: &str) -> String {
    match name {
        "%DEFAULT FOLDER%" => String::new(),
        _ => name.to_string(),
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct HeaderNode {
    size: u32,
    objects_count: u32,
}

fn read_header_node(cursor: &mut Cursor<&[u8]>) -> Result<HeaderNode, Error> {
    let mut buf = [0u8; HEADER_NODE_SIZE];
    read_exact(cursor, &mut buf)?;
    let size = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    // bytes 4-11: 8-byte padding (ignored)
    let objects_count = u32::from_le_bytes(buf[12..16].try_into().unwrap());
    Ok(HeaderNode {
        size,
        objects_count,
    })
}

#[derive(Debug)]
struct NamedNode {
    name: String,
    node_type: u8,
}

fn read_named_node(cursor: &mut Cursor<&[u8]>) -> Result<NamedNode, Error> {
    let mut name_bytes = Vec::new();
    loop {
        let mut pair = [0u8; 2];
        read_exact(cursor, &mut pair)?;
        if pair[0] == 0 && pair[1] == 0 {
            break;
        }
        name_bytes.extend_from_slice(&pair);
    }

    let u16s: Vec<u16> = name_bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let name = String::from_utf16_lossy(&u16s);

    let mut type_buf = [0u8; 1];
    read_exact(cursor, &mut type_buf)?;

    Ok(NamedNode {
        name,
        node_type: type_buf[0],
    })
}

#[derive(Debug)]
struct OptionalFileNode {
    original_size: u32,
    stored_size: u32,
}

fn read_optional_file_node(cursor: &mut Cursor<&[u8]>) -> Result<OptionalFileNode, Error> {
    let mut buf = [0u8; 53];
    read_exact(cursor, &mut buf)?;
    let original_size = u32::from_le_bytes(buf[2..6].try_into().unwrap());
    let stored_size = u32::from_le_bytes(buf[49..53].try_into().unwrap());
    Ok(OptionalFileNode {
        original_size,
        stored_size,
    })
}

fn read_exact(cursor: &mut Cursor<&[u8]>, buf: &mut [u8]) -> Result<(), Error> {
    match cursor.read_exact(buf) {
        Ok(()) => Ok(()),
        Err(_) => Err(Error::UnexpectedEof),
    }
}
