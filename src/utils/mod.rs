use std::fs;
use std::path::Path;

pub mod discovery;
pub mod logging;
pub mod process;
pub mod watcher;

pub fn xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(&xml[start..start + end])
}

pub fn has_enigma(exe_path: &Path) -> bool {
    let Ok(raw) = fs::read(exe_path) else {
        return false;
    };
    let Ok(pe) = evbunpack_rs::enigma::PeInfo::parse(&raw) else {
        return false;
    };
    let names: Vec<&str> = pe.sections.iter().map(|s| s.name.as_str()).collect();
    names.contains(&".enigma1")
}
