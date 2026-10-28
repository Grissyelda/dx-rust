//! dxtool: inspect a Deus Ex install.
//!
//!   dxtool summary <package-or-map>         object counts by class
//!   dxtool extract-src <System dir> <out>   write embedded UnrealScript to <out>/<Pkg>/Classes/<Class>.uc
//!   dxtool actors <map> [ClassFilter]       list actors in a map with location and key properties
//!
//! Extracted source is a LOCAL working copy for porting. Do not commit or redistribute it.

use std::{env, fs, path::Path, process::ExitCode};
use std::collections::HashMap;
use std::io::Write;
use unreal_pkg::objects::{self, pf, Palette, Texture};
use unreal_pkg::props::{self, Value};
use unreal_pkg::{Package, Reader};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Classes that are map plumbing, not placed actors.
const NON_ACTOR: &[&str] = &[
    "Class", "Function", "State", "Struct", "Enum", "TextBuffer", "Polys", "Model", "Level",
    "Texture", "Palette", "Sound", "Music", "Mesh", "LodMesh", "Const", "Package",
];

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("summary") => {
            let path = args.get(1).ok_or("usage: dxtool summary <package>")?;
            let pkg = Package::open(path)?;
            println!(
                "version {} licensee {}: {} names, {} imports, {} exports",
                pkg.version,
                pkg.licensee,
                pkg.names.len(),
                pkg.imports.len(),
                pkg.exports.len()
            );
            let mut counts: Vec<_> = pkg.export_counts().into_iter().collect();
            counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            for (class, n) in counts.iter().take(40) {
                println!("{n:>7}  {class}");
            }
            Ok(())
        }
        Some("extract-src") => {
            let sys = args.get(1).ok_or("usage: dxtool extract-src <System dir> <out dir>")?;
            let out = args.get(2).ok_or("usage: dxtool extract-src <System dir> <out dir>")?;
            let mut total = 0usize;
            let mut entries: Vec<_> = fs::read_dir(sys)?.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("u") {
                    continue;
                }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg").to_string();
                let pkg = match Package::open(&path) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("skip {stem}: {e}");
                        continue;
                    }
                };
                let dir = Path::new(out).join(&stem).join("Classes");
                let mut n = 0;
                for t in pkg.script_texts() {
                    fs::create_dir_all(&dir)?;
                    let bytes: Vec<u8> = t.text.chars().map(|c| c as u32 as u8).collect();
                    fs::write(dir.join(format!("{}.uc", t.owner)), bytes)?;
                    n += 1;
                }
                if n > 0 {
                    println!("{stem}: {n} classes");
                    total += n;
                }
            }
            println!("total: {total} classes");
            Ok(())
        }
        Some("actors") => {
            let path = args.get(1).ok_or("usage: dxtool actors <map> [ClassFilter]")?;
            let filter = args.get(2).map(|s| s.to_ascii_lowercase());
            let pkg = Package::open(path)?;
            let mut shown = 0usize;
            let mut failed = 0usize;
            for e in &pkg.exports {
                let class = pkg.class_name(e.class_index);
                if NON_ACTOR.contains(&class) || e.size == 0 {
                    continue;
                }
                if let Some(f) = &filter {
                    if !class.to_ascii_lowercase().contains(f.as_str()) {
                        continue;
                    }
                }
                let data = pkg.export_data(e);
                let mut r = Reader::new(data, 0);
                if e.flags & props::RF_HAS_STACK != 0 {
                    if props::skip_state_frame(&mut r).is_err() {
                        failed += 1;
                        continue;
                    }
                }
                match props::read_properties(&pkg, &mut r) {
                    Ok(ps) => {
                        let loc = match props::find(&ps, "Location") {
                            Some(Value::Vector(v)) => format!("({:.0}, {:.0}, {:.0})", v[0], v[1], v[2]),
                            _ => "-".to_string(),
                        };
                        let tag = match props::find(&ps, "Tag") {
                            Some(Value::Name(n)) => n.clone(),
                            _ => String::new(),
                        };
                        let event = match props::find(&ps, "Event") {
                            Some(Value::Name(n)) => n.clone(),
                            _ => String::new(),
                        };
                        println!("{:<28} {:<22} {:<26} tag={tag} event={event} props={}", class, e.name, loc, ps.len());
                        shown += 1;
                    }
                    Err(_) => failed += 1,
                }
            }
            eprintln!("{shown} actors listed, {failed} objects failed to decode");
            Ok(())
        }
        Some("models") => {
            let path = args.get(1).ok_or("usage: dxtool models <map>")?;
            let pkg = Package::open(path)?;
            let mut rows = Vec::new();
            for e in &pkg.exports {
                if pkg.class_name(e.class_index) != "Model" {
                    continue;
                }
                match objects::read_model(&pkg, e) {
                    Ok(m) => rows.push((m.nodes.len(), e.name.clone(), m.points.len(), m.surfaces.len(), m.num_zones)),
                    Err(err) => eprintln!("{}: {err}", e.name),
                }
            }
            rows.sort_by(|a, b| b.0.cmp(&a.0));
            for (n, name, pts, surfs, zones) in rows.iter().take(8) {
                println!("{name:<14} nodes={n:<7} points={pts:<7} surfaces={surfs:<6} zones={zones}");
            }
            println!("{} models decoded", rows.len());
            Ok(())
        }
        Some("textures") => {
            let path = args.get(1).ok_or("usage: dxtool textures <package.utx>")?;
            let pkg = Package::open(path)?;
            let mut shown = 0;
            let mut by_format: HashMap<u8, usize> = HashMap::new();
            for (i, e) in pkg.exports.iter().enumerate() {
                if pkg.class_name(e.class_index) != "Texture" {
                    continue;
                }
                match objects::read_texture(&pkg, e) {
                    Ok(t) => {
                        *by_format.entry(t.format).or_default() += 1;
                        if shown < 6 {
                            println!(
                                "{:<40} {}x{} format={} mips={} palette_ref={}",
                                pkg.object_path(i as i32 + 1),
                                t.width(),
                                t.height(),
                                t.format,
                                t.mips.len(),
                                t.palette_ref
                            );
                            shown += 1;
                        }
                    }
                    Err(err) => eprintln!("{}: {err}", e.name),
                }
            }
            println!("formats: {by_format:?}");
            Ok(())
        }
        Some("defaults") => {
            // dxtool defaults <System dir> <Class>
            let dir = args.get(1).ok_or("usage: dxtool defaults <System dir> <Class>")?;
            let class = args.get(2).ok_or("usage: dxtool defaults <System dir> <Class>")?;
            let mut db = unreal_pkg::classes::ClassDb::open(Path::new(dir));
            let d = db.defaults(class);
            if d.is_empty() {
                return Err(format!("class {class} not found or has no defaults").into());
            }
            for p in d {
                if p.index > 0 {
                    println!("{}[{}] = {:?}", p.name, p.index, p.value);
                } else {
                    println!("{} = {:?}", p.name, p.value);
                }
            }
            Ok(())
        }
        Some("level-info") => {
            let root = args.get(1).ok_or("usage: dxtool level-info <game root> <map file>")?;
            let map = args.get(2).ok_or("usage: dxtool level-info <game root> <map file>")?;
            let lvl = dx_level::load_level(Path::new(root), map)?;
            println!("{}: {:?}", lvl.name, lvl.stats);
            println!("bounds (m): {:?} .. {:?}", lvl.bounds_min, lvl.bounds_max);
            println!("player start: {:?}", lvl.player_start);
            for b in &lvl.batches {
                if b.texture.is_none() {
                    println!("untextured batch: {} verts", b.positions.len());
                }
            }
            println!("{} batches, {} textures", lvl.batches.len(), lvl.textures.len());
            Ok(())
        }
        Some("export-level") => {
            let map = args.get(1).ok_or("usage: dxtool export-level <map> <Textures dir> <out dir>")?;
            let tex_dir = args.get(2).ok_or("usage: dxtool export-level <map> <Textures dir> <out dir>")?;
            let out = args.get(3).ok_or("usage: dxtool export-level <map> <Textures dir> <out dir>")?;
            export_level(map, tex_dir, out)
        }
        _ => {
            eprintln!(
                "usage:\n  dxtool summary <package>\n  dxtool extract-src <System dir> <out dir>\n  dxtool actors <map> [ClassFilter]\n  dxtool models <map>\n  dxtool textures <package.utx>\n  dxtool export-level <map> <Textures dir> <out dir>"
            );
            Ok(())
        }
    }
}

/// Loads texture packages on demand and decodes textures to RGBA.
struct TexLib {
    dir: String,
    pkgs: HashMap<String, Option<Package>>,
}

impl TexLib {
    fn new(dir: &str) -> Self {
        Self { dir: dir.to_string(), pkgs: HashMap::new() }
    }

    fn package(&mut self, name: &str) -> Option<&Package> {
        let key = name.to_ascii_lowercase();
        if !self.pkgs.contains_key(&key) {
            let p = Package::open(Path::new(&self.dir).join(format!("{name}.utx"))).ok();
            self.pkgs.insert(key.clone(), p);
        }
        self.pkgs.get(&key).and_then(|p| p.as_ref())
    }

    /// Resolve a dotted path like `Package.Group.Name` to a decoded texture and its RGBA pixels.
    fn load(&mut self, path: &str) -> Option<(Texture, Vec<u8>)> {
        let (pkg_name, rest) = path.split_once('.')?;
        let pkg = self.package(pkg_name)?;
        let e = pkg.find_export_by_path(rest)?;
        let tex = objects::read_texture(pkg, e).ok()?;
        let palette: Option<Palette> = if tex.palette_ref > 0 {
            let pe = pkg.exports.get(tex.palette_ref as usize - 1)?;
            objects::read_palette(pkg, pe).ok()
        } else if tex.palette_ref < 0 {
            let full = pkg.object_path(tex.palette_ref);
            let (ppkg, prest) = full.split_once('.')?;
            let ppkg = ppkg.to_string();
            let prest = prest.to_string();
            let pk = self.package(&ppkg)?;
            let pe = pk.find_export_by_path(&prest)?;
            objects::read_palette(pk, pe).ok()
        } else {
            None
        };
        let rgba = tex.to_rgba8(palette.as_ref())?;
        Some((tex, rgba))
    }
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn export_level(map: &str, tex_dir: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let pkg = Package::open(map)?;
    // The level's own BSP is the Model with the most nodes.
    let mut best: Option<objects::Model> = None;
    for e in &pkg.exports {
        if pkg.class_name(e.class_index) != "Model" {
            continue;
        }
        if let Ok(m) = objects::read_model(&pkg, e) {
            if best.as_ref().map(|b| m.nodes.len() > b.nodes.len()).unwrap_or(true) {
                best = Some(m);
            }
        }
    }
    let model = best.ok_or("no Model in map")?;
    println!("level model {}: {} nodes, {} surfaces, {} points", model.name, model.nodes.len(), model.surfaces.len(), model.points.len());

    fs::create_dir_all(Path::new(out).join("tex"))?;
    let mut lib = TexLib::new(tex_dir);
    let mut tex_ids: HashMap<i32, Option<(String, u32, u32)>> = HashMap::new();

    let mut obj = std::io::BufWriter::new(fs::File::create(Path::new(out).join("level.obj"))?);
    let mut mtl = std::io::BufWriter::new(fs::File::create(Path::new(out).join("level.mtl"))?);
    writeln!(obj, "mtllib level.mtl")?;

    let (mut verts_written, mut faces, mut skipped_flags, mut missing_tex) = (0usize, 0usize, 0usize, 0usize);
    let mut used_mats: Vec<String> = Vec::new();
    let mut by_surface: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, n) in model.nodes.iter().enumerate() {
        by_surface.entry(n.surf).or_default().push(i);
    }
    let mut surf_ids: Vec<_> = by_surface.keys().copied().collect();
    surf_ids.sort();

    for sid in surf_ids {
        let Some(surf) = model.surfaces.get(sid as usize) else { continue };
        if surf.poly_flags & (pf::INVISIBLE | pf::PORTAL) != 0 {
            skipped_flags += by_surface[&sid].len();
            continue;
        }
        let info = tex_ids
            .entry(surf.material)
            .or_insert_with(|| {
                let path = pkg.object_path(surf.material);
                let (t, rgba) = lib.load(&path)?;
                let name = path.replace('.', "_");
                write_png(&Path::new(out).join("tex").join(format!("{name}.png")), t.width(), t.height(), &rgba).ok()?;
                Some((name, t.width(), t.height()))
            })
            .clone();
        let (mat, tw, th) = match info {
            Some(i) => i,
            None => {
                missing_tex += by_surface[&sid].len();
                ("missing".to_string(), 256, 256)
            }
        };
        if !used_mats.contains(&mat) {
            writeln!(mtl, "newmtl {mat}\nKd 1 1 1\nmap_Kd tex/{mat}.png\n")?;
            used_mats.push(mat.clone());
        }
        let base = model.points[surf.p_base as usize];
        let tu = model.vectors[surf.v_texture_u as usize];
        let tv = model.vectors[surf.v_texture_v as usize];
        writeln!(obj, "usemtl {mat}")?;
        for &ni in &by_surface[&sid] {
            let n = &model.nodes[ni];
            if n.num_vertices < 3 {
                continue;
            }
            let mut idx = Vec::new();
            for k in 0..n.num_vertices as usize {
                let v = &model.vertices[n.vert_pool as usize + k];
                let p = model.points[v.vertex as usize];
                let d = [p[0] - base[0], p[1] - base[1], p[2] - base[2]];
                let u = (d[0] * tu[0] + d[1] * tu[1] + d[2] * tu[2] + surf.pan_u as f32) / tw as f32;
                let vv = (d[0] * tv[0] + d[1] * tv[1] + d[2] * tv[2] + surf.pan_v as f32) / th as f32;
                writeln!(obj, "v {} {} {}", p[0], p[2], p[1])?;
                writeln!(obj, "vt {} {}", u, -vv)?;
                verts_written += 1;
                idx.push(verts_written);
            }
            for k in 1..idx.len() - 1 {
                writeln!(obj, "f {0}/{0} {1}/{1} {2}/{2}", idx[0], idx[k], idx[k + 1])?;
                faces += 1;
            }
        }
    }
    println!("wrote {faces} triangles, {verts_written} vertices, {} textures", used_mats.len());
    println!("skipped {skipped_flags} invisible/portal faces, {missing_tex} faces with unresolved textures");
    Ok(())
}
