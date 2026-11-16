//! Gives `Spider.exe` (and `SpiderBackground.exe`) an icon and file properties.
//!
//! The icon is drawn here, at every size Windows asks for: a white spider hanging from its
//! thread on a rounded square in Spider's gradient. Shapes are distance fields, so each size is
//! drawn sharp (no blurry downscale). The pictures go into a `.res` file written by hand (icon
//! group + version information) that the MSVC linker takes as an input: no resource compiler
//! needed. Other toolchains simply build without an icon.

use std::path::PathBuf;

const SIZES: [u32; 6] = [256, 64, 48, 32, 24, 16];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(windows && msvc) {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let pictures: Vec<(u32, Vec<u8>)> = SIZES
        .iter()
        .map(|&size| (size, png(size, size, &draw(size))))
        .collect();
    let res = out.join("spider.res");
    std::fs::write(&res, resources(&pictures)).expect("the resource file is written");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

// --- Drawing ----------------------------------------------------------------

fn to_linear(channel: f32) -> f32 {
    let c = channel / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(channel: f32) -> f32 {
    let c = channel.clamp(0.0, 1.0);
    255.0 * if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Spider's Venom gradient (pink, violet, blue), mixed in linear light.
fn gradient(t: f32) -> [f32; 3] {
    const STOPS: [[f32; 3]; 3] = [[255.0, 60.0, 172.0], [150.0, 82.0, 255.0], [43.0, 134.0, 255.0]];
    let t = t.clamp(0.0, 1.0) * 2.0;
    let (a, b, k) = if t <= 1.0 { (STOPS[0], STOPS[1], t) } else { (STOPS[1], STOPS[2], t - 1.0) };
    let mut colour = [0.0; 3];
    for i in 0..3 {
        let mixed = to_linear(a[i]) + (to_linear(b[i]) - to_linear(a[i])) * k;
        colour[i] = to_srgb(mixed);
    }
    colour
}

fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Distance to a rounded square centred in the unit square.
fn rounded_square(x: f32, y: f32, half: f32, radius: f32) -> f32 {
    let qx = (x - 0.5).abs() - half + radius;
    let qy = (y - 0.5).abs() - half + radius;
    length(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

/// Distance to a segment.
fn segment(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (px, py) = (x - a.0, y - a.1);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let h = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    length(px - dx * h, py - dy * h)
}

/// The spider's legs on the left side (the right side is their mirror): from the body, up to
/// the knee, down to the foot.
const LEGS: [[(f32, f32); 3]; 4] = [
    [(0.46, 0.44), (0.33, 0.29), (0.23, 0.37)],
    [(0.45, 0.48), (0.27, 0.41), (0.15, 0.51)],
    [(0.45, 0.54), (0.27, 0.58), (0.17, 0.70)],
    [(0.46, 0.59), (0.33, 0.70), (0.26, 0.84)],
];

/// The spider's distance at a point, in units of the picture (negative inside).
fn spider(x: f32, y: f32, stroke: f32) -> f32 {
    let head = length(x - 0.5, y - 0.405) - 0.07;
    let body = length(x - 0.5, y - 0.575) - 0.125;
    let thread = segment(x, y, (0.5, 0.0), (0.5, 0.36)) - stroke * 0.32;
    // Legs are drawn on the folded half: the spider is symmetric.
    let folded = 0.5 - (x - 0.5).abs();
    let mut legs = f32::MAX;
    for leg in LEGS {
        legs = legs.min(segment(folded, y, leg[0], leg[1]).min(segment(folded, y, leg[1], leg[2])));
    }
    head.min(body).min(thread).min(legs - stroke * 0.5)
}

/// The icon at `size` pixels, as RGBA rows.
fn draw(size: u32) -> Vec<u8> {
    let scale = size as f32;
    // Strokes keep at least about a pixel and a half, so the legs survive at 16 px.
    let stroke = (0.034_f32).max(1.5 / scale);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for row in 0..size {
        for column in 0..size {
            let x = (column as f32 + 0.5) / scale;
            let y = (row as f32 + 0.5) / scale;
            let square = (0.5 - rounded_square(x, y, 0.47, 0.2) * scale).clamp(0.0, 1.0);
            let glyph = (0.5 - spider(x, y, stroke) * scale).clamp(0.0, 1.0) * square;
            let mut colour = gradient((x + y) * 0.5);
            // A soft light from the top left, a little shade at the bottom right.
            let light = 1.0 + 0.12 * (1.0 - (x + y));
            for channel in &mut colour {
                *channel = (*channel * light).min(255.0);
            }
            for channel in &mut colour {
                *channel += (255.0 - *channel) * glyph;
            }
            rgba.extend([
                colour[0].round() as u8,
                colour[1].round() as u8,
                colour[2].round() as u8,
                (square * 255.0).round() as u8,
            ]);
        }
    }
    rgba
}

// --- PNG ----------------------------------------------------------------------

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 4 + 1) * height) as usize);
    for row in rgba.chunks((width * 4) as usize) {
        raw.push(0);
        raw.extend(row);
    }
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9));
    chunk(&mut out, b"IEND", &[]);
    out
}

// --- The .res file ------------------------------------------------------------

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;

fn align(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

/// One resource: a 32-byte header (numeric type and name), then the data, padded.
fn resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, language: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(kind.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(name.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(flags.to_le_bytes());
    out.extend(language.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(data);
    align(out);
}

fn resources(pictures: &[(u32, Vec<u8>)]) -> Vec<u8> {
    const ENGLISH: u16 = 0x0409;
    let mut out = Vec::new();
    // The empty entry every .res file starts with.
    resource(&mut out, 0, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend(0u16.to_le_bytes());
    group.extend(1u16.to_le_bytes());
    group.extend((pictures.len() as u16).to_le_bytes());
    for (index, (size, data)) in pictures.iter().enumerate() {
        let id = index as u16 + 1;
        resource(&mut out, RT_ICON, id, 0x1010, ENGLISH, data);
        let side = if *size >= 256 { 0 } else { *size as u8 };
        group.extend([side, side, 0, 0]);
        group.extend(1u16.to_le_bytes());
        group.extend(32u16.to_le_bytes());
        group.extend((data.len() as u32).to_le_bytes());
        group.extend(id.to_le_bytes());
    }
    resource(&mut out, RT_GROUP_ICON, 1, 0x1030, ENGLISH, &group);
    resource(&mut out, RT_VERSION, 1, 0x0030, ENGLISH, &version_info());
    out
}

/// A version-information node: length, value length, type, key, value, children.
fn node(key: &str, value: &[u8], value_length: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0, 0];
    out.extend(value_length.to_le_bytes());
    out.extend(u16::from(text).to_le_bytes());
    for unit in key.encode_utf16().chain([0]) {
        out.extend(unit.to_le_bytes());
    }
    align(&mut out);
    out.extend(value);
    for child in children {
        align(&mut out);
        out.extend(child);
    }
    let length = out.len() as u16;
    out[..2].copy_from_slice(&length.to_le_bytes());
    out
}

fn text(key: &str, value: &str) -> Vec<u8> {
    let units: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    node(key, &bytes, units.len() as u16, true, &[])
}

fn version_info() -> Vec<u8> {
    let part = |name: &str| -> u32 {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut fixed = Vec::new();
    for value in [
        0xFEEF_04BD,         // signature
        0x0001_0000,         // structure version
        (major << 16) | minor, // file version
        patch << 16,
        (major << 16) | minor, // product version
        patch << 16,
        0x3F, // flags mask
        0,    // flags
        0x0004_0004, // NT, Windows32
        1,    // application
        0,
        0,
        0,
    ] {
        fixed.extend(u32::to_le_bytes(value));
    }
    let strings = node(
        "040904B0",
        &[],
        0,
        true,
        &[
            text("FileDescription", "Spider — Discord Rich Presence sans token"),
            text("FileVersion", &version),
            text("ProductName", "Spider"),
            text("ProductVersion", &version),
            text("LegalCopyright", "Licence MIT"),
        ],
    );
    let string_info = node("StringFileInfo", &[], 0, true, &[strings]);
    let translation = node("Translation", &[0x09, 0x04, 0xB0, 0x04], 4, false, &[]);
    let var_info = node("VarFileInfo", &[], 0, true, &[translation]);
    node("VS_VERSION_INFO", &fixed, 52, false, &[string_info, var_info])
}
