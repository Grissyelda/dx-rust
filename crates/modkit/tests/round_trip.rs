//! Make the three kinds of download this unpacks (a .7z, a .zip, and a 7-Zip self-extracting .exe = a program with
//! a 7z after it) from a small fake mod tree, then sniff, unpack and install each one.

use modkit::archive::{self, Kind};
use std::io::Write;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("modkit-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A Nameless-Mod-like tree inside a top folder, plus an LDDP-like mod beside it.
fn fake_tree(root: &Path) {
    for (rel, body) in [
        ("TNM/System/TNM.u", "tnm"),
        ("TNM/TNM2/Maps/20_Entry.dx", "map"),
        ("TNM/Help/readme.txt", "hello"),
        ("LayD/System/FemJC.u", "lddp"),
    ] {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body.repeat(1000)).unwrap();
    }
}

fn make_zip(src: &Path, out: &Path) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(out).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for rel in ["TNM/System/TNM.u", "TNM/TNM2/Maps/20_Entry.dx", "TNM/Help/readme.txt", "LayD/System/FemJC.u"] {
        // Windows separators, as zips made on Windows sometimes carry.
        z.start_file(rel.replace('/', "\\"), opts).unwrap();
        z.write_all(&std::fs::read(src.join(rel)).unwrap()).unwrap();
    }
    z.finish().unwrap();
}

#[test]
fn archives_and_installers_unpack() {
    let work = tmp("work");
    let src = work.join("src");
    fake_tree(&src);
    let sz = work.join("The Nameless Mod.7z");
    sevenz_rust2::compress_to_path(&src, &sz).unwrap();
    let zp = work.join("tnm_full.zip");
    make_zip(&src, &zp);
    // A self-extractor: a program, padding, then the 7z (what 7-Zip's SFX modules write).
    let exe = work.join("The_Nameless_Mod_Installer.exe");
    let mut bytes = b"MZ".to_vec();
    bytes.extend(std::iter::repeat_n(0x90u8, 300_000));
    bytes.extend(b";!@Install@!UTF-8!\nTitle=\"Test\"\n;!@InstallEnd@!");
    bytes.extend(std::fs::read(&sz).unwrap());
    std::fs::write(&exe, bytes).unwrap();
    // A program with no archive inside.
    let plain = work.join("tool.exe");
    std::fs::write(&plain, [b"MZ".as_slice(), &[0u8; 5000]].concat()).unwrap();

    assert_eq!(archive::sniff(&sz).unwrap(), Kind::SevenZ { offset: 0 });
    assert_eq!(archive::sniff(&zp).unwrap(), Kind::Zip { offset: 0 });
    assert!(matches!(archive::sniff(&exe).unwrap(), Kind::SevenZ { offset } if offset > 300_000));
    assert_eq!(archive::sniff(&plain).unwrap(), Kind::Exe);

    for (i, f) in [&sz, &zp, &exe].into_iter().enumerate() {
        let dest = work.join(format!("out{i}"));
        let kind = archive::sniff(f).unwrap();
        let mut last = 0.0;
        let n = archive::extract(f, kind, &dest, &mut |p, _| last = p).unwrap();
        assert_eq!(n, 4, "{}", f.display());
        assert!(last > 0.99);
        assert_eq!(std::fs::read_to_string(dest.join("TNM/Help/readme.txt")).unwrap(), "hello".repeat(1000));
        let mods = modkit::install::find_mods(&dest);
        let ids: Vec<&str> = mods.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&"tnm") && ids.contains(&"lddp"), "{ids:?}");
        assert!(mods.iter().find(|(id, _)| *id == "tnm").unwrap().1.ends_with("TNM"));
    }

    // A full install into a data folder: files land in Mods/tnm and launcher.txt gets the lines, keeping others.
    let data = work.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("launcher.txt"), "GamePath=C:\\Games\\Deus Ex\nModOn.lddp=0\n").unwrap();
    unreal_pkg::paths::set_data_dir(data.clone());
    let done = modkit::install::install(&exe, &mut |_, _| {}).unwrap();
    assert_eq!(done.folder, data.join("Mods").join("tnm"));
    let txt = std::fs::read_to_string(data.join("launcher.txt")).unwrap();
    assert!(txt.starts_with("GamePath=C:\\Games\\Deus Ex\n"), "{txt}");
    assert!(txt.contains("ModOn.lddp=1") && txt.contains("TNMPath="), "{txt}");
    assert_eq!(txt.matches("ModOn.lddp").count(), 1, "{txt}");
    // Installing again replaces the copy; removing deletes our folder and the lines.
    modkit::install::install(&exe, &mut |_, _| {}).unwrap();
    modkit::install::remove("tnm", &done.folder.join("TNM")).unwrap();
    assert!(!done.folder.exists());
    assert!(!std::fs::read_to_string(data.join("launcher.txt")).unwrap().contains("TNMPath"));
    // A plain program is refused with instructions, and nothing is left behind.
    let e = modkit::install::install(&plain, &mut |_, _| {}).unwrap_err();
    assert!(e.contains("Windows program"), "{e}");
    let _ = std::fs::remove_dir_all(&work);
}
