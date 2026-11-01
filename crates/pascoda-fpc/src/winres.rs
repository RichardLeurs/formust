//! Windows resources for an exported program: a manifest (so the controls get the current look instead
//! of the Windows 95 one), the application icon and the version information (Explorer's Details tab). Written as a `.res` file that the compiler links with
//! `{$R pascoda.res}`; no resource compiler is needed.
//!
//! The `.res` format: a 32-byte empty resource, then each resource as a header (sizes, type, name, flags)
//! followed by its data, every part padded to 4 bytes.

use std::path::Path;
use std::process::Command;

/// What a resource type or name is: a number or a string.
enum Id<'a> {
    Num(u16),
    Str(&'a str),
}

impl Id<'_> {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Id::Num(n) => {
                out.extend(0xFFFFu16.to_le_bytes());
                out.extend(n.to_le_bytes());
            }
            Id::Str(s) => {
                for u in s.to_uppercase().encode_utf16().chain([0]) {
                    out.extend(u.to_le_bytes());
                }
            }
        }
    }
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;

fn pad4(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn entry(out: &mut Vec<u8>, kind: Id, name: Id, data: &[u8]) {
    let start = out.len();
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(0u32.to_le_bytes()); // header size, filled in below
    kind.write(out);
    name.write(out);
    pad4(out);
    out.extend(0u32.to_le_bytes()); // data version
    out.extend(0x0030u16.to_le_bytes()); // memory flags: moveable, pure
    out.extend(0u16.to_le_bytes()); // language: neutral
    out.extend(0u32.to_le_bytes()); // version
    out.extend(0u32.to_le_bytes()); // characteristics
    let header = (out.len() - start) as u32;
    out[start + 4..start + 8].copy_from_slice(&header.to_le_bytes());
    out.extend(data);
    pad4(out);
}

/// What the version information says: the program's name and its version as typed (`1.2`, `2.0.1.5`).
pub struct Version<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

/// The four numbers of a version; a missing or non-numeric part is 0 (`1.2` is 1.2.0.0).
pub fn version_parts(version: &str) -> [u16; 4] {
    let mut parts = [0u16; 4];
    for (slot, text) in parts.iter_mut().zip(version.trim().split('.')) {
        *slot = text.trim().parse().unwrap_or(0);
    }
    parts
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// One node of the version resource: length, value length, type (1 = text, 0 = binary), key, padding, value and
/// children, each part on a 4-byte boundary. The length counts the node's own bytes without the padding after it.
fn node(key: &str, value_len: u16, text: bool, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0u8; 6];
    b.extend(utf16z(key));
    pad4(&mut b);
    b.extend(value);
    for child in children {
        pad4(&mut b);
        b.extend(child);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b[2..4].copy_from_slice(&value_len.to_le_bytes());
    b[4..6].copy_from_slice(&(text as u16).to_le_bytes());
    b
}

/// The `VS_VERSIONINFO` data: the fixed file and product version, and the strings Explorer shows (language
/// English, code page Unicode: `040904B0`).
pub fn version_block(v: &Version) -> Vec<u8> {
    let [a, b, c, d] = version_parts(v.version);
    let text = format!("{a}.{b}.{c}.{d}");
    let mut fixed = Vec::new();
    for word in [
        0xFEEF04BDu32,                         // signature
        0x0001_0000,                           // structure version
        (a as u32) << 16 | b as u32,           // file version, high
        (c as u32) << 16 | d as u32,           // file version, low
        (a as u32) << 16 | b as u32,           // product version, high
        (c as u32) << 16 | d as u32,           // product version, low
        0x3F,                                  // flags mask
        0,                                     // flags
        0x0004_0004,                           // Windows NT, 32 bit
        1,                                     // an application
        0,                                     // no subtype
        0,                                     // date, high
        0,                                     // date, low
    ] {
        fixed.extend(word.to_le_bytes());
    }
    let exe = format!("{}.exe", v.name);
    let strings: Vec<Vec<u8>> = [
        ("FileDescription", v.name),
        ("FileVersion", text.as_str()),
        ("InternalName", v.name),
        ("OriginalFilename", exe.as_str()),
        ("ProductName", v.name),
        ("ProductVersion", text.as_str()),
    ]
    .iter()
    .map(|(k, val)| node(k, val.encode_utf16().count() as u16 + 1, true, &utf16z(val), &[]))
    .collect();
    let table = node("040904B0", 0, true, &[], &strings);
    let string_info = node("StringFileInfo", 0, true, &[], &[table]);
    let translation = node("Translation", 4, false, &0x04B0_0409u32.to_le_bytes(), &[]);
    let var_info = node("VarFileInfo", 0, true, &[], &[translation]);
    node("VS_VERSION_INFO", fixed.len() as u16, false, &fixed, &[string_info, var_info])
}

/// The manifest: version 6 of the common controls (the current look), no elevation.
pub fn manifest(name: &str) -> String {
    let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let name = if name.is_empty() { "Program".to_string() } else { name };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="Pascoda.{name}" type="win32"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
</assembly>
"#
    )
}

/// The `.res` file: the manifest, the version information and, when there is one, the icon (a PNG of at most 256 x 256 pixels, which
/// Windows has read inside a resource since Vista). The icon group is named `MAINICON`, as Lazarus names it,
/// so the LCL also uses it for the program's windows.
pub fn res_file(manifest: &str, icon_png: Option<&[u8]>, version: &Version) -> Vec<u8> {
    let mut out = vec![0u8; 0];
    out.extend([0, 0, 0, 0, 0x20, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
    out.extend([0u8; 16]);
    entry(&mut out, Id::Num(RT_MANIFEST), Id::Num(1), manifest.as_bytes());
    entry(&mut out, Id::Num(RT_VERSION), Id::Num(1), &version_block(version));
    if let Some(png) = icon_png {
        entry(&mut out, Id::Num(RT_ICON), Id::Num(1), png);
        let mut group = Vec::new();
        group.extend(0u16.to_le_bytes()); // reserved
        group.extend(1u16.to_le_bytes()); // type: icon
        group.extend(1u16.to_le_bytes()); // one image
        group.extend([0, 0, 0, 0]); // 256 x 256 (0 = 256), no palette
        group.extend(1u16.to_le_bytes()); // planes
        group.extend(32u16.to_le_bytes()); // bits per pixel
        group.extend((png.len() as u32).to_le_bytes());
        group.extend(1u16.to_le_bytes()); // the RT_ICON id
        entry(&mut out, Id::Num(RT_GROUP_ICON), Id::Str("MAINICON"), &group);
    }
    out
}

/// The icon (a `.png` or `.icns`) as a 256 x 256 PNG, converted with `sips`.
pub fn icon_png(icon: &Path, scratch: &Path) -> std::io::Result<Vec<u8>> {
    let out = scratch.join("icon256.png");
    let st = Command::new("sips")
        .args(["-s", "format", "png", "-z", "256", "256"])
        .arg(icon)
        .arg("--out")
        .arg(&out)
        .output()?;
    if !st.status.success() {
        return Err(std::io::Error::other(format!(
            "could not read the icon: {}",
            String::from_utf8_lossy(&st.stderr).trim()
        )));
    }
    std::fs::read(&out)
}

/// The lines a Windows program's source gets after its `program` line: its resource file, and the PE OS and
/// subsystem versions when a minimum Windows is given (`6.1` is Windows 7, `10.0` Windows 10).
pub fn directive_lines(res: &str, min_windows: Option<&str>) -> Vec<String> {
    let mut lines = vec![format!("{{$R {res}}}")];
    if let Some(v) = min_windows {
        lines.push(format!("{{$SETPEOSVERSION {v}}}"));
        lines.push(format!("{{$SETPESUBSYSVERSION {v}}}"));
    }
    lines
}

/// `source` with `{$R <res>}` added after its `program` line, or `None` when that cannot be done (no
/// `program` header, or the program already pulls in resources of its own with `{$R`).
pub fn with_resource(source: &str, res: &str) -> Option<String> {
    with_directives(source, &directive_lines(res, None))
}

/// `source` with `lines` added, in order, after its `program` line; `None` as for [`with_resource`].
pub fn with_directives(source: &str, lines: &[String]) -> Option<String> {
    let lower = source.to_ascii_lowercase();
    if lower.contains("{$r ") || lower.contains("{$resource ") {
        // The program has its own resources (and a manifest of its own, most likely).
        return None;
    }
    // The first `program` that starts a line, outside a comment.
    let mut at = 0;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let t = line.trim_start().to_ascii_lowercase();
        if t.starts_with("program ") || t.starts_with("program\t") {
            let end = line.find(';').map(|i| offset + i + 1)?;
            at = end;
            break;
        }
        offset += line.len();
    }
    if at == 0 {
        return None;
    }
    let added: String = lines.iter().map(|l| format!("\n{l}")).collect();
    Some(format!("{}{}{}", &source[..at], added, &source[at..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
    }

    /// Walks the resources after the empty one: (type, name, data length).
    fn walk(res: &[u8]) -> Vec<(String, String, usize)> {
        let mut found = Vec::new();
        let mut i = 32;
        let id = |at: &mut usize| -> String {
            let first = u16::from_le_bytes([res[*at], res[*at + 1]]);
            if first == 0xFFFF {
                let n = u16::from_le_bytes([res[*at + 2], res[*at + 3]]);
                *at += 4;
                n.to_string()
            } else {
                let mut s = String::new();
                loop {
                    let u = u16::from_le_bytes([res[*at], res[*at + 1]]);
                    *at += 2;
                    if u == 0 {
                        break;
                    }
                    s.push(char::from_u32(u as u32).unwrap());
                }
                s
            }
        };
        while i < res.len() {
            let (size, header) = (u32_at(res, i) as usize, u32_at(res, i + 4) as usize);
            let mut at = i + 8;
            let kind = id(&mut at);
            let name = id(&mut at);
            found.push((kind, name, size));
            assert_eq!(header % 4, 0, "the header is padded");
            i += header + size.div_ceil(4) * 4;
        }
        assert_eq!(i, res.len(), "the file ends on a resource boundary");
        found
    }

    #[test]
    fn the_res_file_holds_the_manifest_and_the_icon_in_the_right_format() {
        let m = manifest("My App!");
        assert!(m.contains("Pascoda.MyApp") && m.contains("Common-Controls") && m.contains("6.0.0.0"));
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        let v = Version { name: "My App", version: "1.2" };
        let res = res_file(&m, Some(&png), &v);
        assert_eq!(&res[..8], [0, 0, 0, 0, 0x20, 0, 0, 0], "starts with the empty resource");
        assert_eq!(
            walk(&res),
            [
                ("24".into(), "1".into(), m.len()),
                ("16".into(), "1".into(), version_block(&v).len()),
                ("3".into(), "1".into(), png.len()),
                ("14".into(), "MAINICON".into(), 20),
            ]
        );
        // without an icon: the manifest and the version
        assert_eq!(walk(&res_file(&m, None, &v)).len(), 2);
    }

    /// The header of the node at `at`: key, value length, type, where the value starts, where the node ends.
    fn read(b: &[u8], at: usize) -> (String, u16, u16, usize, usize) {
        let len = u16::from_le_bytes([b[at], b[at + 1]]) as usize;
        let value_len = u16::from_le_bytes([b[at + 2], b[at + 3]]);
        let kind = u16::from_le_bytes([b[at + 4], b[at + 5]]);
        let mut key = String::new();
        let mut i = at + 6;
        loop {
            let u = u16::from_le_bytes([b[i], b[i + 1]]);
            i += 2;
            if u == 0 {
                break;
            }
            key.push(char::from_u32(u as u32).unwrap());
        }
        let value_at = i.div_ceil(4) * 4;
        assert_eq!(at % 4, 0, "{key} starts on a 4-byte boundary");
        (key, value_len, kind, value_at, at + len)
    }

    /// Every (key, text value) of the string table, checking that the lengths add up all the way down.
    fn strings(b: &[u8]) -> Vec<(String, String)> {
        let (key, value_len, kind, value_at, end) = read(b, 0);
        assert_eq!((key.as_str(), value_len, kind), ("VS_VERSION_INFO", 52, 0));
        assert_eq!(end, b.len(), "the root length is the whole block");
        assert_eq!(u32::from_le_bytes(b[value_at..value_at + 4].try_into().unwrap()), 0xFEEF04BD);
        let mut at = (value_at + 52).div_ceil(4) * 4;
        let (info, _, _, table_at, info_end) = read(b, at);
        assert_eq!(info, "StringFileInfo");
        at = table_at;
        let (table, _, _, mut entry, table_end) = read(b, at);
        assert_eq!(table, "040904B0");
        assert_eq!(table_end, info_end);
        let mut found = Vec::new();
        while entry < table_end {
            let (k, chars, kind, value_at, end) = read(b, entry);
            assert_eq!(kind, 1, "text");
            let units: Vec<u16> = (0..chars as usize)
                .map(|n| u16::from_le_bytes([b[value_at + 2 * n], b[value_at + 2 * n + 1]]))
                .collect();
            assert_eq!(units.last(), Some(&0), "the value ends with a null");
            assert_eq!(end, value_at + 2 * chars as usize, "the length ends with the value");
            found.push((k, String::from_utf16(&units[..units.len() - 1]).unwrap()));
            entry = end.div_ceil(4) * 4;
        }
        // then the translation
        let (var, _, _, var_at, _) = read(b, info_end.div_ceil(4) * 4);
        assert_eq!(var, "VarFileInfo");
        let (tr, vl, _, tr_value, tr_end) = read(b, var_at);
        assert_eq!((tr.as_str(), vl), ("Translation", 4));
        assert_eq!(u32::from_le_bytes(b[tr_value..tr_value + 4].try_into().unwrap()), 0x04B0_0409);
        assert_eq!(tr_end, b.len());
        found
    }

    #[test]
    fn the_version_block_has_the_numbers_and_the_strings_explorer_shows() {
        assert_eq!(version_parts("1.2"), [1, 2, 0, 0]);
        assert_eq!(version_parts(" 2.0.1.5 "), [2, 0, 1, 5]);
        assert_eq!(version_parts("x.9"), [0, 9, 0, 0]);
        assert_eq!(version_parts(""), [0, 0, 0, 0]);
        let b = version_block(&Version { name: "Ünïcode App", version: "3.4.5" });
        let found = strings(&b);
        let get = |k: &str| found.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("ProductName"), Some("Ünïcode App"));
        assert_eq!(get("FileDescription"), Some("Ünïcode App"));
        assert_eq!(get("FileVersion"), Some("3.4.5.0"));
        assert_eq!(get("ProductVersion"), Some("3.4.5.0"));
        assert_eq!(get("OriginalFilename"), Some("Ünïcode App.exe"));
        // the fixed numbers: 3.4 and 5.0, file and product
        let words: Vec<u32> = (0..13).map(|n| u32::from_le_bytes(b[40 + 4 * n..44 + 4 * n].try_into().unwrap())).collect();
        assert_eq!(&words[2..6], [0x0003_0004, 0x0005_0000, 0x0003_0004, 0x0005_0000]);
    }

    #[test]
    fn a_minimum_windows_version_adds_the_pe_version_directives_after_the_resource() {
        assert_eq!(directive_lines("r.res", None), ["{$R r.res}"]);
        let lines = directive_lines("r.res", Some("6.1"));
        assert_eq!(lines, ["{$R r.res}", "{$SETPEOSVERSION 6.1}", "{$SETPESUBSYSVERSION 6.1}"]);
        let out = with_directives("program hello;\nbegin end.\n", &lines).unwrap();
        assert_eq!(
            out,
            "program hello;\n{$R r.res}\n{$SETPEOSVERSION 6.1}\n{$SETPESUBSYSVERSION 6.1}\nbegin end.\n"
        );
    }

    #[test]
    fn the_resource_line_goes_after_the_program_header_and_only_when_there_are_no_resources_of_the_programs_own() {
        let src = "program hello;\n{$mode objfpc}\nuses Forms;\nbegin end.\n";
        let out = with_resource(src, "pascoda.res").unwrap();
        assert!(out.starts_with("program hello;\n{$R pascoda.res}\n{$mode objfpc}"), "{out}");
        assert!(with_resource("// program x;\nbegin end.", "r.res").is_none(), "a comment is not a header");
        assert!(with_resource("program p;\n{$R *.res}\nbegin end.", "r.res").is_none(), "its own resources win");
        assert!(with_resource("library l;\nbegin end.", "r.res").is_none());
    }
}
